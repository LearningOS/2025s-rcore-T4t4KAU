# LAB 1 实验总结

在本实验中要求记录syscall的调用次数

核心思路是扩展`TaskManager`

```rust
/// Inner of Task Manager
pub struct TaskManagerInner {
    /// task list
    tasks: [TaskControlBlock; MAX_APP_NUM],
    /// id of current `Running` task
    current_task: usize,

    task_info_map: [TaskInfo; MAX_APP_NUM],
}
```

添加初始化

```rust
lazy_static! {
    /// Global variable: TASK_MANAGER
    pub static ref TASK_MANAGER: TaskManager = {
        let num_app = get_num_app();
        let mut tasks = [TaskControlBlock {
            task_cx: TaskContext::zero_init(),
            task_status: TaskStatus::UnInit,
        }; MAX_APP_NUM];
        for (i, task) in tasks.iter_mut().enumerate() {
            task.task_cx = TaskContext::goto_restore(init_app_cx(i));
            task.task_status = TaskStatus::Ready;
        }
        let task_info_map = [TaskInfo {
            syscall_count: [0; MAX_SYSCALL_NUM],
        }; MAX_APP_NUM];

        TaskManager {
            num_app,
            inner: unsafe {
                UPSafeCell::new(TaskManagerInner {
                    tasks,
                    current_task: 0,
                    task_info_map,
                })
            },
        }
    };
}
```

扩充函数支持

```rust
impl TaskManager {
    /// Run the first task in task list.
    ///
    /// Generally, the first task in task list is an idle task (we call it zero process later).
    /// But in ch3, we load apps statically, so the first task is a real app.
    fn run_first_task(&self) -> ! {
        let mut inner = self.inner.exclusive_access();
        let task0 = &mut inner.tasks[0];
        task0.task_status = TaskStatus::Running;
        let next_task_cx_ptr = &task0.task_cx as *const TaskContext;
        drop(inner);
        let mut _unused = TaskContext::zero_init();
        // before this, we should drop local variables that must be dropped manually
        unsafe {
            __switch(&mut _unused as *mut TaskContext, next_task_cx_ptr);
        }
        panic!("unreachable in run_first_task!");
    }

    /// Change the status of current `Running` task into `Ready`.
    fn mark_current_suspended(&self) {
        let mut inner = self.inner.exclusive_access();
        let current = inner.current_task;
        inner.tasks[current].task_status = TaskStatus::Ready;
    }

    /// Change the status of current `Running` task into `Exited`.
    fn mark_current_exited(&self) {
        let mut inner = self.inner.exclusive_access();
        let current = inner.current_task;
        inner.tasks[current].task_status = TaskStatus::Exited;
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

}
```

```rust
/// increment syscall count of current task
pub fn inc_current_syscall_count(syscall_id: usize) {
    TASK_MANAGER.inc_current_syscall_count(syscall_id);
}


/// get current task's syscall count
pub fn get_current_syscall_count(syscall_id: usize) -> usize {
    TASK_MANAGER.get_current_syscall_count(syscall_id)
}
```

修改syscall

```rust
/// handle syscall exception with `syscall_id` and other arguments
pub fn syscall(syscall_id: usize, args: [usize; 3]) -> isize {
    inc_current_syscall_count(syscall_id);

    let res= match syscall_id {
        SYSCALL_WRITE => {
            sys_write(args[0], args[1] as *const u8, args[2])
        },
        SYSCALL_EXIT => {
            sys_exit(args[0] as i32)
        },
        SYSCALL_YIELD => {
            sys_yield()
        },
        SYSCALL_GET_TIME => {
            sys_get_time(args[0] as *mut TimeVal, args[1])
        },

        SYSCALL_TRACE => {
            sys_trace(args[0], args[1], args[2])
        }

        _ => panic!("Unsupported syscall_id: {}", syscall_id),
    };

    res
}

```

完成sys_trace:

```rust
pub fn sys_trace(_trace_request: usize, _id: usize, _data: usize) -> isize {
    trace!("kernel: sys_trace");

    match _trace_request {
        0 => {
            let addr = _id as *const u8;
            let data = unsafe { *addr };
            data as isize
        }

        1 => {
            let addr = _id as *mut u8;
            unsafe { *addr = _data as u8 }
            0
        }

        2 => {
            get_current_syscall_count(_id) as isize
        }

        _ => -1,
    }
}
```

