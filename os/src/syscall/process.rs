//! Process management syscalls
//!
use alloc::sync::Arc;

use crate::{
    fs::{open_file, OpenFlags},
    mm::{translated_refmut, translated_str, translated_byte_buffer, MapPermission, VirtPageNum},
    task::{
        add_task, current_task, current_user_token, exit_current_and_run_next,
        suspend_current_and_run_next,
    },
    config::{PAGE_SIZE, PAGE_SIZE_BITS},
    timer::get_time_us
};

use core::slice::from_raw_parts;

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

pub fn sys_exit(exit_code: i32) -> ! {
    trace!("kernel:pid[{}] sys_exit", current_task().unwrap().pid.0);
    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit!");
}

pub fn sys_yield() -> isize {
    //trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

pub fn sys_getpid() -> isize {
    trace!("kernel: sys_getpid pid:{}", current_task().unwrap().pid.0);
    current_task().unwrap().pid.0 as isize
}

pub fn sys_fork() -> isize {
    trace!("kernel:pid[{}] sys_fork", current_task().unwrap().pid.0);
    let current_task = current_task().unwrap();
    let new_task = current_task.fork();
    let new_pid = new_task.pid.0;
    // modify trap context of new_task, because it returns immediately after switching
    let trap_cx = new_task.inner_exclusive_access().get_trap_cx();
    // we do not have to move to next instruction since we have done it before
    // for child process, fork returns 0
    trap_cx.x[10] = 0;
    // add new task to scheduler
    add_task(new_task);
    new_pid as isize
}

pub fn sys_exec(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_exec", current_task().unwrap().pid.0);
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(app_inode) = open_file(path.as_str(), OpenFlags::RDONLY) {
        let all_data = app_inode.read_all();
        let task = current_task().unwrap();
        task.exec(all_data.as_slice());
        0
    } else {
        -1
    }
}

/// If there is not a child process whose pid is same as given, return -1.
/// Else if there is a child process but it is still running, return -2.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    //trace!("kernel: sys_waitpid");
    let task = current_task().unwrap();
    // find a child process

    // ---- access current PCB exclusively
    let mut inner = task.inner_exclusive_access();
    if !inner
        .children
        .iter()
        .any(|p| pid == -1 || pid as usize == p.getpid())
    {
        return -1;
        // ---- release current PCB
    }
    let pair = inner.children.iter().enumerate().find(|(_, p)| {
        // ++++ temporarily access child PCB exclusively
        p.inner_exclusive_access().is_zombie() && (pid == -1 || pid as usize == p.getpid())
        // ++++ release child PCB
    });
    if let Some((idx, _)) = pair {
        let child = inner.children.remove(idx);
        // confirm that child will be deallocated after being removed from children list
        assert_eq!(Arc::strong_count(&child), 1);
        let found_pid = child.getpid();
        // ++++ temporarily access child PCB exclusively
        let exit_code = child.inner_exclusive_access().exit_code;
        // ++++ release child PCB
        *translated_refmut(inner.memory_set.token(), exit_code_ptr) = exit_code;
        found_pid as isize
    } else {
        -2
    }
    // ---- release current PCB automatically
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(_ts: *mut TimeVal, _tz: usize) -> isize {
    let us = get_time_us();
    let tv_size = core::mem::size_of::<TimeVal>();
    let dst_vec = translated_byte_buffer(current_user_token(), _ts as *const u8, tv_size);

    let ref time_val = TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };

    let src_ptr = time_val as *const TimeVal;
    for (idx, dst) in dst_vec.into_iter().enumerate() {
        let unit_len = dst.len();
        unsafe {
            dst.copy_from_slice(from_raw_parts(
                src_ptr.wrapping_byte_add(idx * unit_len) as *const u8, unit_len));
        }   
    }

    0
}

/// YOUR JOB: Implement mmap.
pub fn sys_mmap(_start: usize, _len: usize, _prot: usize) -> isize {
    // check low 12 bits
    if _start & (1 << PAGE_SIZE_BITS) - 1 != 0 {
        println!("kernel: invalid start address, start = {:#x}", _start);
        return -1;
    }

    let page_num = (_len + PAGE_SIZE - 1) / PAGE_SIZE;
    println!("kernel: attempt to allocate {} pages", page_num);

    if _prot & !0x7 != 0 || _prot & 0x7 == 0 {
        println!("kernel: invalid mask, _prot = {:#x}", _prot);
        return -1;
    }

    let mut perm = MapPermission::U;
    if _prot & (0x1 << 0) != 0 {
        perm = perm | MapPermission::R;
    }

    if _prot & (0x1 << 1) != 0 {
        perm = perm | MapPermission::W;
    }

    if _prot & (0x1 << 2) != 0 {
        perm = perm | MapPermission::X;
    }


    let start_vpn: VirtPageNum = (_start / PAGE_SIZE).into();
    if let Some(cur_task) = current_task() {
        let mut _current = cur_task.inner_exclusive_access();
        for i in 0..page_num {
            let vpn: VirtPageNum = (start_vpn.0 + i).into();
            if _current.memory_set.check_vpn_mapped(vpn) {
                println!("kernel: virtual page {:#x} has been mapped", vpn.0);
                return -1;
            }
        }
        
        _current.memory_set.map_pages(_start.into(), page_num, perm);
        println!("kernel: map virtual pages, start_va = {:#x}, end_va = {:#x}", _start, _start + page_num * PAGE_SIZE);

        return 0;
    } else {
        println!("kernel: failed to get current task");
        return -1;
    }
}

/// YOUR JOB: Implement munmap.
pub fn sys_munmap(_start: usize, _len: usize) -> isize {
    // check low 12 bits
    if _start & (1 << PAGE_SIZE_BITS) - 1 != 0 {
        println!("kernel: invalid start address, start = {:#x}", _start);
        return -1;
    }

    let page_num = (_len + PAGE_SIZE - 1) / PAGE_SIZE;
    println!("kernel: attempt to unmap {} pages", page_num);

    if let Some(cur_task) = current_task() {
        let mut _current = cur_task.inner_exclusive_access();
        for i in 0..page_num {
            let addr = _start + PAGE_SIZE * i;
            let vpn: VirtPageNum = (addr / PAGE_SIZE).into();
            if !_current.memory_set.check_vpn_mapped(vpn) {
                println!("kernel: virtual page {:#x} hasn't been mapped", vpn.0);
                return -1;
            }

            _current.memory_set.unmap_one_page(vpn);
            println!("kernel: unmap one page, vpn = {:#x}", vpn.0);
        }

        return 0;
    } else {
        println!("kernel: failed to get current task");
        return -1;
    }
}

/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel:pid[{}] sys_sbrk", current_task().unwrap().pid.0);
    if let Some(old_brk) = current_task().unwrap().change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}

/// YOUR JOB: Implement spawn.
/// HINT: fork + exec =/= spawn
pub fn sys_spawn(_path: *const u8) -> isize {
    let token = current_user_token();
    let path = translated_str(token, _path);
    if let Some(app_inode) = open_file(path.as_str(), OpenFlags::RDONLY) {
        let all_data = app_inode.read_all();
        let new_task = current_task().unwrap().spawn(&all_data);
        let new_task_id = new_task.getpid();
        println!("sys_spawn: create process {} on {}", new_task_id, path.as_str());
        add_task(new_task);

        new_task_id as isize
    } else {
        return -1;
    }
}

// YOUR JOB: Set task priority.
pub fn sys_set_priority(_prio: isize) -> isize {
    if _prio <= 1 {
        return -1;
    }

    if let Some(cur_task) = current_task() {
        cur_task.set_priority(_prio);
        
        return _prio;
    } else {
        println!("kernel: failed to get current task");
        return -1;
    }
}
