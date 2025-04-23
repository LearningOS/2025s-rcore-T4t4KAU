//! File and filesystem-related syscalls
use crate::fs::{get_ino, link_file, open_file, stat_file, unlink_file, File, OpenFlags, Stat};
use crate::mm::{translated_byte_buffer, translated_str, UserBuffer};
use crate::task::{current_task, current_user_token};

pub fn sys_write(fd: usize, buf: *const u8, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_write", current_task().unwrap().pid.0);
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if let Some(file) = &inner.fd_table[fd] {
        if !file.writable() {
            return -1;
        }
        let file = file.clone();
        // release current task TCB manually to avoid multi-borrow
        drop(inner);
        file.write(UserBuffer::new(translated_byte_buffer(token, buf, len))) as isize
    } else {
        -1
    }
}

pub fn sys_read(fd: usize, buf: *const u8, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_read", current_task().unwrap().pid.0);
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if let Some(file) = &inner.fd_table[fd] {
        let file = file.clone();
        if !file.readable() {
            return -1;
        }
        // release current task TCB manually to avoid multi-borrow
        drop(inner);
        trace!("kernel: sys_read .. file.read");
        file.read(UserBuffer::new(translated_byte_buffer(token, buf, len))) as isize
    } else {
        -1
    }
}

pub fn sys_open(path: *const u8, flags: u32) -> isize {
    trace!("kernel:pid[{}] sys_open", current_task().unwrap().pid.0);
    let task = current_task().unwrap();
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(inode) = open_file(path.as_str(), OpenFlags::from_bits(flags).unwrap()) {
        let mut inner = task.inner_exclusive_access();
        let fd = inner.alloc_fd();

        let block_info = inode.block_info();
        inner.fd_table[fd] = Some(inode);
        inner.fd_map.push((fd, block_info));

        fd as isize
    } else {
        -1
    }
}

pub fn sys_close(fd: usize) -> isize {
    trace!("kernel:pid[{}] sys_close", current_task().unwrap().pid.0);
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if inner.fd_table[fd].is_none() {
        return -1;
    }

    let idx = inner.fd_map.iter().position(|x| x.0 == fd).unwrap();
    inner.fd_map.remove(idx);
    inner.fd_table[fd].take();

    0
}

/// YOUR JOB: Implement fstat.
pub fn sys_fstat(_fd: usize, _st: *mut Stat) -> isize {
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    let token = inner.get_user_token();

    // check legality
    if _fd >= inner.fd_table.len() {
        return -1;
    }
    if inner.fd_table[_fd].is_none() {
        return -1;
    }

    if let Some(file) = &inner.fd_table[_fd] {
        println!("kernel: attempt to get status, fd = {}", _fd);
        let _file =  file.clone();
        let (block_id, block_offset) = file.block_info();
        let ino = get_ino(block_id, block_offset).unwrap();

        println!("kernel: attempt to get status, fd = {}, ino = {}", _fd, ino);
        if let Some(stat) = &stat_file(ino) {
            let st = translated_byte_buffer(token, _st as *const u8, core::mem::size_of::<Stat>());
            let stat_ptr = stat as *const _ as *const u8;
            for (idx, byte) in st.into_iter().enumerate() {
                unsafe {
                    byte.copy_from_slice(core::slice::from_raw_parts(stat_ptr.wrapping_byte_add(idx), byte.len()));
                }
            }

            println!("get status, fd = {}, ino = {}, nlink = {}", _fd, ino, stat.nlink);

            return 0;
        } else {
            println!("failed to get status, fd = {}, ino = {}", _fd, ino);
            return -1;
        }
    } else {
        println!("failed to access fd:{} in fd_table", _fd);
        return -1;
    }
}

/// YOUR JOB: Implement linkat.
pub fn sys_linkat(_old_name: *const u8, _new_name: *const u8) -> isize {
    let token = current_user_token();
    let old_name = translated_str(token, _old_name);
    let new_name = translated_str(token, _new_name);

    println!("kernel: attempt to link {} to {}", new_name, old_name);
    
    let (x, id) = link_file(old_name.as_str(), new_name.as_str());
    println!("kernel: link {} to {} on {}", new_name, old_name, id);

    return x;
}

/// YOUR JOB: Implement unlinkat.
pub fn sys_unlinkat(_name: *const u8) -> isize {
    let token = current_user_token();
    let name = translated_str(token, _name);

    println!("kernel: attempt to unlink {}", name);
    
    // let mut flag = false;
    // let block_info = get_block_by_name(name.as_str());
    // let cur_task = current_task().unwrap();
    // let inner = cur_task.inner_exclusive_access();

    // println!("kernel: block info {},{} on {}", block_info.0, block_info.1, name);

    // for (_, entry) in inner.fd_map.iter().enumerate() {
    //     let info =  entry.1;
    //     println!("kernel: find block {},{}", info.0, info.1);
    //     if block_info.0 == info.0 && block_info.1 == info.1 {
    //         flag = true;
    //         break;
    //     }
    // }

    // if !flag {
    //     println!("kernel: invalid file {}", name);
    //     return -1;
    // }


    let ret = unlink_file(name.as_str());
    if ret == -1 {
        println!("kernel: failed to unlink file {}", name);
    }

    ret
}
