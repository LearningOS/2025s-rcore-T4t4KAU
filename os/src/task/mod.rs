//! Task management implementation
//!
//! Everything about task management, like starting and switching tasks is
//! implemented here.
//!
//! A single global instance of [`TaskManager`] called `TASK_MANAGER` controls
//! all the tasks in the operating system.
//!
//! Be careful when you see `__switch` ASM function in `switch.S`. Control flow around this function
//! might not be what you expect.

mod context;
mod switch;
#[allow(clippy::module_inception)]
mod task;

use crate::loader::{get_app_data, get_num_app};
use crate::mm::{MapPermission, PTEFlags, PhysPageNum, VirtAddr, VirtPageNum};
use crate::sync::UPSafeCell;
use crate::trap::TrapContext;
use alloc::vec::Vec;
use lazy_static::*;
use switch::__switch;
use task::TaskInfo;
pub use task::{TaskControlBlock, TaskStatus};

pub use context::TaskContext;

use crate::config::MAX_SYSCALL_NUM;

/// The task manager, where all the tasks are managed.
///
/// Functions implemented on `TaskManager` deals with all task state transitions
/// and task context switching. For convenience, you can find wrappers around it
/// in the module level.
///
/// Most of `TaskManager` are hidden behind the field `inner`, to defer
/// borrowing checks to runtime. You can see examples on how to use `inner` in
/// existing functions on `TaskManager`.
pub struct TaskManager {
    /// total number of tasks
    num_app: usize,
    /// use inner value to get mutable access
    inner: UPSafeCell<TaskManagerInner>,
}

/// The task manager inner in 'UPSafeCell'
struct TaskManagerInner {
    /// task list
    tasks: Vec<TaskControlBlock>,
    /// id of current `Running` task
    current_task: usize,

    task_info_map: Vec<TaskInfo>
}

lazy_static! {
    /// a `TaskManager` global instance through lazy_static!
    pub static ref TASK_MANAGER: TaskManager = {
        println!("init TASK_MANAGER");
        let num_app = get_num_app();
        println!("num_app = {}", num_app);
        let mut tasks: Vec<TaskControlBlock> = Vec::new();
        let mut task_info_map: Vec<TaskInfo> = Vec::new();
        for i in 0..num_app {
            tasks.push(TaskControlBlock::new(get_app_data(i), i));
            task_info_map.push(TaskInfo {
                syscall_count: [0; MAX_SYSCALL_NUM],
            });
        }
        TaskManager {
            num_app,
            inner: unsafe {
                UPSafeCell::new(TaskManagerInner {
                    tasks,
                    current_task: 0,
                    task_info_map
                })
            },
        }
    };
}

impl TaskManager {
    /// Run the first task in task list.
    ///
    /// Generally, the first task in task list is an idle task (we call it zero process later).
    /// But in ch4, we load apps statically, so the first task is a real app.
    fn run_first_task(&self) -> ! {
        let mut inner = self.inner.exclusive_access();
        let next_task = &mut inner.tasks[0];
        next_task.task_status = TaskStatus::Running;
        let next_task_cx_ptr = &next_task.task_cx as *const TaskContext;
        drop(inner);
        let mut _unused = TaskContext::zero_init();
        // before this, we should drop local variables that must be dropped manually
        unsafe {
            __switch(&mut _unused as *mut _, next_task_cx_ptr);
        }
        panic!("unreachable in run_first_task!");
    }

    /// Change the status of current `Running` task into `Ready`.
    fn mark_current_suspended(&self) {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].task_status = TaskStatus::Ready;
    }

    /// Change the status of current `Running` task into `Exited`.
    fn mark_current_exited(&self) {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].task_status = TaskStatus::Exited;
    }

    /// Find next task to run and return task id.
    ///
    /// In this case, we only return the first `Ready` task in task list.
    fn find_next_task(&self) -> Option<usize> {
        let inner = self.inner.exclusive_access();
        let current = inner.current_task;
        (current + 1..current + self.num_app + 1)
            .map(|id| id % self.num_app)
            .find(|id| inner.tasks[*id].task_status == TaskStatus::Ready)
    }

    /// Get the current 'Running' task's token.
    fn get_current_token(&self) -> usize {
        let inner = self.inner.exclusive_access();
        inner.tasks[inner.current_task].get_user_token()
    }

    /// Get the current 'Running' task's trap contexts.
    fn get_current_trap_cx(&self) -> &'static mut TrapContext {
        let inner = self.inner.exclusive_access();
        inner.tasks[inner.current_task].get_trap_cx()
    }

    /// Change the current 'Running' task's program break
    pub fn change_current_program_brk(&self, size: i32) -> Option<usize> {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].change_program_brk(size)
    }

    /// Switch current `Running` task to the task we have found,
    /// or there is no `Ready` task and we can exit with all applications completed
    fn run_next_task(&self) {
        if let Some(next) = self.find_next_task() {
            let mut inner = self.inner.exclusive_access();
            let current = inner.current_task;
            inner.tasks[next].task_status = TaskStatus::Running;
            inner.current_task = next;
            let current_task_cx_ptr = &mut inner.tasks[current].task_cx as *mut TaskContext;
            let next_task_cx_ptr = &inner.tasks[next].task_cx as *const TaskContext;
            drop(inner);
            // before this, we should drop local variables that must be dropped manually
            unsafe {
                __switch(current_task_cx_ptr, next_task_cx_ptr);
            }
            // go back to user mode
        } else {
            panic!("All applications completed!");
        }
    }

    fn inc_current_syscall_count(&self, syscall_id: usize) {
        let mut inner = self.inner.exclusive_access();
        let current = inner.current_task;
        inner.task_info_map[current].syscall_count[syscall_id] += 1;
    }

    fn get_current_syscall_count(&self, syscall_id: usize) -> usize {
        let inner = self.inner.exclusive_access();
        let current = inner.current_task;
        inner.task_info_map[current].syscall_count[syscall_id]
    }

    fn check_vpn_readable(&self, vpn: VirtPageNum) -> bool {
        let inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        
        inner.tasks[cur].memory_set.check_vpn_readable(vpn)
    }

    fn check_vpn_writable(&self, vpn: VirtPageNum) -> bool {
        let inner = self.inner.exclusive_access();
        let cur = inner.current_task;

        inner.tasks[cur].memory_set.check_vpn_writable(vpn)
    }

    fn alloc_free_page(&self, vpn: VirtPageNum, ppn: PhysPageNum, flags: PTEFlags) {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;

        inner.tasks[cur].memory_set.alloc_free_page(vpn, ppn, flags);
    }

    fn dealloc_free_page(&self, vpn: VirtPageNum) {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;

        inner.tasks[cur].memory_set.dealloc_free_page(vpn);
    }

    fn map_pages(&self, _start: VirtAddr, page_num: usize, _perm: MapPermission) {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;

        inner.tasks[cur].memory_set.map_pages(_start, page_num, _perm);
    }

    fn unmap_one_page(&self, vpn: VirtPageNum) {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;

        inner.tasks[cur].memory_set.unmap_one_page(vpn);
    }

    fn check_page_mapped(&self, vpn: VirtPageNum) -> bool {
        let inner = self.inner.exclusive_access();
        let cur = inner.current_task;

        inner.tasks[cur].memory_set.check_page_mapped(vpn)
    }

    fn write_byte_to_page(&self, addr: VirtAddr, data: u8) {
        let inner = self.inner.exclusive_access();
        let cur = inner.current_task;

        inner.tasks[cur].memory_set.write_byte_to_page(addr, data);
    }

    fn read_byte_from_page(&self, addr: VirtAddr) -> isize {
        let inner = self.inner.exclusive_access();
        let cur = inner.current_task;

        inner.tasks[cur].memory_set.read_byte_from_page(addr)
    }
}

/// Run the first task in task list.
pub fn run_first_task() {
    TASK_MANAGER.run_first_task();
}

/// Switch current `Running` task to the task we have found,
/// or there is no `Ready` task and we can exit with all applications completed
fn run_next_task() {
    TASK_MANAGER.run_next_task();
}

/// Change the status of current `Running` task into `Ready`.
fn mark_current_suspended() {
    TASK_MANAGER.mark_current_suspended();
}

/// Change the status of current `Running` task into `Exited`.
fn mark_current_exited() {
    TASK_MANAGER.mark_current_exited();
}

/// Suspend the current 'Running' task and run the next task in task list.
pub fn suspend_current_and_run_next() {
    mark_current_suspended();
    run_next_task();
}

/// Exit the current 'Running' task and run the next task in task list.
pub fn exit_current_and_run_next() {
    mark_current_exited();
    run_next_task();
}

/// Get the current 'Running' task's token.
pub fn current_user_token() -> usize {
    TASK_MANAGER.get_current_token()
}

/// Get the current 'Running' task's trap contexts.
pub fn current_trap_cx() -> &'static mut TrapContext {
    TASK_MANAGER.get_current_trap_cx()
}

/// Change the current 'Running' task's program break
pub fn change_program_brk(size: i32) -> Option<usize> {
    TASK_MANAGER.change_current_program_brk(size)
}

/// increment syscall count of current task
pub fn inc_current_syscall_count(syscall_id: usize) {
    TASK_MANAGER.inc_current_syscall_count(syscall_id);
}

/// get current task's syscall count
pub fn get_current_syscall_count(syscall_id: usize) -> usize {
    TASK_MANAGER.get_current_syscall_count(syscall_id)
}

/// check virtual address if readable
pub fn check_vpn_readable(vpn: VirtPageNum) -> bool {
    TASK_MANAGER.check_vpn_readable(vpn)
}

/// check virtual address if writable 
pub fn check_vpn_writable(vpn: VirtPageNum) -> bool {
    TASK_MANAGER.check_vpn_writable(vpn)
}

/// alloc_free_page
pub fn alloc_free_page(vpn: VirtPageNum, ppn: PhysPageNum, flags: PTEFlags) {
    TASK_MANAGER.alloc_free_page(vpn, ppn, flags);
}

///
pub fn dealloc_free_page(vpn: VirtPageNum) {
    TASK_MANAGER.dealloc_free_page(vpn);
}

/// check virtual page if mapped
pub fn check_page_mapped(vpn: VirtPageNum) -> bool {
    TASK_MANAGER.check_page_mapped(vpn)
}

/// write one byte to page
pub fn write_byte_to_page(addr: VirtAddr, data: u8) {
    TASK_MANAGER.write_byte_to_page(addr, data);
}

/// read one byte from page
pub fn read_byte_from_page(addr: VirtAddr) -> isize {
    TASK_MANAGER.read_byte_from_page(addr)
}

/// map memory pages
pub fn map_memory_pages(start_va: VirtAddr, page_num: usize, map_perm: MapPermission) {
    TASK_MANAGER.map_pages(start_va, page_num, map_perm);
}

/// unmap one memory page
pub fn unmap_one_memory_page(vpn: VirtPageNum) {
    TASK_MANAGER.unmap_one_page(vpn);
}