use crate::sync::{Condvar, Mutex, MutexBlocking, MutexSpin, Semaphore};
use crate::syscall::process::sys_getpid;
use crate::task::{block_current_and_run_next, current_process, current_task};
use crate::timer::{add_timer, get_time_ms};
use alloc::sync::Arc;
use alloc::vec::Vec;
/// sleep syscall
pub fn sys_sleep(ms: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_sleep",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let expire_ms = get_time_ms() + ms;
    let task = current_task().unwrap();
    add_timer(expire_ms, task);
    block_current_and_run_next();
    0
}
/// mutex create syscall
pub fn sys_mutex_create(blocking: bool) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mutex: Option<Arc<dyn Mutex>> = if !blocking {
        Some(Arc::new(MutexSpin::new()))
    } else {
        Some(Arc::new(MutexBlocking::new()))
    };
    let mut process_inner = process.inner_exclusive_access();
    if let Some(id) = process_inner
        .mutex_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.mutex_list[id] = mutex;
        process_inner.mutex_max[id] = 1;

        id as isize
    } else {
        process_inner.mutex_list.push(mutex);
        process_inner.mutex_max.push(1);

        for alloc_vec in process_inner.mutex_alloc.values_mut() {
            alloc_vec.push(0);
        }

        process_inner.mutex_list.len() as isize - 1
    }
}
/// mutex lock syscall
pub fn sys_mutex_lock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_lock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );

    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();

    println!("kernel: attempt to alloc mutex {}", mutex_id);
    if process_inner.enable {
        if !process_inner.check_mutex_deadlock(mutex_id) {
            println!(
                "kernel: check deadlock of mutex: reject to alloc {}",
                mutex_id
            );
            return -0xDEAD;
        }
    }

    println!(
        "kernel: check deadlock of mutex: allow to alloc {}",
        mutex_id
    );
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());

    // update allocation
    let mut alloc_vec = process_inner.mutex_alloc[&mutex_id].clone();
    alloc_vec[mutex_id] = 0;
    process_inner.mutex_alloc.insert(sys_getpid() as usize, alloc_vec);

    drop(process_inner);
    drop(process);
    mutex.lock();

    println!("kernel: allocate mutex {}", mutex_id);
    0
}
/// mutex unlock syscall
pub fn sys_mutex_unlock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_unlock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();

    println!("kernel: attempt to unlock mutex {}", mutex_id);

    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());

    // update allocation
    let mut alloc_vec = process_inner.mutex_alloc[&mutex_id].clone();
    alloc_vec[mutex_id] = 1;
    process_inner.mutex_alloc.insert(sys_getpid() as usize, alloc_vec);

    drop(process_inner);
    drop(process);
    mutex.unlock();

    println!("kernel: unlock mutex {}", mutex_id);
    0
}
/// semaphore create syscall
pub fn sys_semaphore_create(res_count: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );

    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .semaphore_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.semaphore_list[id] = Some(Arc::new(Semaphore::new(res_count)));
        id
    } else {
        process_inner
            .semaphore_list
            .push(Some(Arc::new(Semaphore::new(res_count))));

        process_inner.semaphore_max.push(res_count);
        process_inner.semaphore_available.push(res_count);

        // update allocation matrix
        for alloc_vec in process_inner.semaphore_alloc.values_mut() {
            alloc_vec.push(0);
        }

        process_inner.semaphore_list.len() - 1

    };

    println!("kernel: create semaphore {}, res_count = {}", id, res_count);

    id as isize
}


/// semaphore up syscall
pub fn sys_semaphore_up(sem_id: usize) -> isize {
    let cur_task = current_task()
        .unwrap()
        .inner_exclusive_access()
        .res
        .as_ref()
        .unwrap()
        .tid;
    
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();

    if !process_inner.semaphore_alloc.contains_key(&cur_task) {
        let mut alloc_vec = Vec::new();
        alloc_vec.resize(process_inner.semaphore_list.len(), 0);
        process_inner.semaphore_alloc.insert(cur_task, alloc_vec);
    }

    println!("kernel: {} attempt to dealloc semaphore {}", cur_task, sem_id);

    // decrease allocation of current task
    process_inner.dec_semaphore_alloc(sem_id, cur_task);

    // increase available num of current task
    process_inner.inc_semaphore_available(sem_id);

    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    
    drop(process_inner);
    sem.up();

    println!("kernel: {} dealloc semaphore {}", cur_task, sem_id);

    0
}
/// semaphore down syscall
pub fn sys_semaphore_down(sem_id: usize) -> isize {
    let cur_task = current_task()
        .unwrap()
        .inner_exclusive_access()
        .res
        .as_ref()
        .unwrap()
        .tid;

    
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();

    if !process_inner.semaphore_alloc.contains_key(&cur_task) {
        let mut alloc_vec = Vec::new();
        alloc_vec.resize(process_inner.semaphore_list.len(), 0);
        process_inner.semaphore_alloc.insert(cur_task, alloc_vec);
    }

    println!("kernel: {} attempt to alloc semaphore {}", cur_task, sem_id);
    if process_inner.enable {
        if !process_inner.check_semaphore_deadlock(sem_id) {
            println!(
                "kernel: check deadlock of semaphore: reject {} to alloc {}",
                cur_task, sem_id
            );
            return -0xDEAD;
        }
    }

    // increase allocation of current task
    process_inner.inc_semaphore_alloc(sem_id, cur_task);

    // decrease available num of currentc task
    process_inner.dec_semaphore_available(sem_id);

    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());

    drop(process_inner);
    sem.down();

    println!("kernel: allocate {} semaphore {}", cur_task, sem_id);
    0
}
/// condvar create syscall
pub fn sys_condvar_create() -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .condvar_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.condvar_list[id] = Some(Arc::new(Condvar::new()));
        id
    } else {
        process_inner
            .condvar_list
            .push(Some(Arc::new(Condvar::new())));
        process_inner.condvar_list.len() - 1
    };
    id as isize
}
/// condvar signal syscall
pub fn sys_condvar_signal(condvar_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_signal",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    drop(process_inner);
    condvar.signal();
    0
}
/// condvar wait syscall
pub fn sys_condvar_wait(condvar_id: usize, mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_wait",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    condvar.wait(mutex);
    0
}
/// enable deadlock detection syscall
///
/// YOUR JOB: Implement deadlock detection, but might not all in this syscall
pub fn sys_enable_deadlock_detect(_enabled: usize) -> isize {
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();

    if _enabled == 1 {
        process_inner.enable_deadlock_detect();
    } else if _enabled == 0 {
        process_inner.disable_deadlock_detect();
    } else {
        return -1;
    }

    return 0;
}
