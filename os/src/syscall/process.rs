//! Process management syscalls
use core::slice::from_raw_parts;

use crate::{config::{PAGE_SIZE, PAGE_SIZE_BITS}, mm::{check_enough, translated_byte_buffer, MapPermission, VirtPageNum}, task::{change_program_brk, check_page_mapped, check_vpn_readable, check_vpn_writable, current_user_token, exit_current_and_run_next, get_current_syscall_count, map_memory_pages, read_byte_from_page, suspend_current_and_run_next, unmap_one_memory_page, write_byte_to_page}, timer::get_time_us};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(_ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time: _ts = {:#x}, _tz = {:#x}", _ts as usize, _tz);

    let us = get_time_us();
    let _ts_ptr = _ts as *const u8;
    let mut bytes_vec = translated_byte_buffer(current_user_token(), _ts_ptr, _tz);
    
    let tv = &mut TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_1000_1000,
    };

    // println!("kernel get time: sec = {:#x} usec = {:#x}", tv.sec, tv.usec);

    let tv_ptr = tv as *const TimeVal as *const u8;
    let tv_bytes: &[u8] = unsafe {
        from_raw_parts(tv_ptr, _tz)
    };
    
    if bytes_vec.len() == 1 {
        assert_eq!(bytes_vec[0].len(), tv_bytes.len());
        for i in 0..tv_bytes.len() {
            bytes_vec[0][i] = tv_bytes[i]
        }
    }

    if bytes_vec.len() == 2 {
        assert_eq!(bytes_vec[0].len() + bytes_vec[1].len(), tv_bytes.len());
        for i in 0..bytes_vec[0].len() {
            bytes_vec[0][i] = tv_bytes[i];
        }

        for i in 0..bytes_vec[1].len() {
            bytes_vec[1][i] = tv_bytes[i];
        }
    }


    0
}

/// TODO: Finish sys_trace to pass testcases
/// HINT: You might reimplement it with virtual memory management.
pub fn sys_trace(_trace_request: usize, _id: usize, _data: usize) -> isize {
    trace!("kernel: sys_trace");

    match _trace_request {
        0 => {
            let vpn: VirtPageNum = (_id / PAGE_SIZE).into();
            println!("kernel: try to read data from address {:#x}, vpn = {:#x}", _id, vpn.0);
            if !check_vpn_readable(vpn) {
                println!("kernel: virtual page {:#x} is not readable", vpn.0);
                return -1;
            }
            
            println!("kernel: virtual page {:#x} is readable", vpn.0);
            let data = read_byte_from_page(_id.into());
            println!("kernel: read data {:#x} from address {:#x}", data, _id);

            data as isize
        }

        1 => {
            let vpn: VirtPageNum = (_id / PAGE_SIZE).into();
            println!("kernel: try to write data {:#x} to address {:#x}", _data, vpn.0);
            if !check_vpn_writable(vpn) {
                println!("kernel: address {:#x} is not writable", _id);
                return -1;
            }

            println!("kernel: page {:#x} is writable", vpn.0);
            write_byte_to_page(_id.into(), _data as u8);
            println!("kernel: writed data {:#x} to address {:#x}", _data, _id);

            0
        }

        2 => {
            let cnt = get_current_syscall_count(_id) as isize;
            println!("kernel: syscall {:#x} count {:#x}", _id, cnt);
            cnt
        }

        _ => -1,
    }
}

// YOUR JOB: Implement mmap.
pub fn sys_mmap(_start: usize, _len: usize, _prot: usize) -> isize {
    println!("kernel: sys_mmap: _start = {:#x}, _len = {}, _prot = {:#x}", _start, _len, _prot);

    if _start & (1 << PAGE_SIZE_BITS) - 1 != 0 {
        println!("kernel: invalid start address, start = {:#x}", _start);
        return -1;
    }

    let page_num = (_len + PAGE_SIZE - 1) / PAGE_SIZE;
    println!("kernel: try to allocate {} pages", page_num);
    if !check_enough(page_num) {
        println!("kernel: no free page to allocate.");
        return -1;
    }

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
    for i in 0..page_num {
        let vpn: VirtPageNum = (start_vpn.0 + i).into();
        if check_page_mapped(vpn) {
            println!("kernel: virtual page {:#x} has been mapped", vpn.0);
            return -1;
        }
    }

    map_memory_pages(_start.into(), page_num, perm);
    println!("kernel: map virtual pages, start_va = {:#x}, end_va = {:#x}", _start, _start + page_num * PAGE_SIZE);

    0
}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(_start: usize, _len: usize) -> isize {
    println!("kernel: sys_munmap: _start = {:#x}, _len = {}", _start, _len);

    // check low 12 bits
    if _start & (1 << PAGE_SIZE_BITS) - 1 != 0 {
        println!("kernel: invalid start address, start = {:#x}", _start);
        return -1;
    }

    let page_num = (_len + PAGE_SIZE - 1) / PAGE_SIZE;
    println!("kernel: try to free page {}", page_num);

    // free memory page
    for i in 0..page_num {
        let addr = _start + PAGE_SIZE * i;
        let vpn: VirtPageNum = (addr / PAGE_SIZE).into();
        if !check_page_mapped(vpn) {
            println!("kernel: virtual page {:#x} hasn't been allocated", vpn.0);
            return -1;
        }

        unmap_one_memory_page(vpn);
        println!("kernel: free one page, vpn = {:#x}", vpn.0);
    }

    0
}
/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
