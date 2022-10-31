use crate::*;

use std::sync::atomic::{AtomicU32, Ordering};

// it is an unstable feature in std::sync::atomic
// https://doc.rust-lang.org/std/sync/atomic/struct.AtomicU32.html#method.from_mut
fn atomicu32_from_mut(v: &mut u32) -> &mut AtomicU32 {
    use core::mem::align_of;
    let [] = [(); align_of::<AtomicU32>() - align_of::<u32>()];
    unsafe { &mut *(v as *mut u32 as *mut AtomicU32) }
}

#[inline]
fn atomic_producer(q: *mut rxe_queue_buf) -> &'static mut AtomicU32 {
    let q = unsafe { &mut *q };
    atomicu32_from_mut(&mut q.producer_index)
}

#[inline]
fn atomic_conumer(q: *mut rxe_queue_buf) -> &'static mut AtomicU32 {
    let q = unsafe { &mut *q };
    atomicu32_from_mut(&mut q.consumer_index)
}

#[inline]
pub fn queue_empty(q: *mut rxe_queue_buf) -> bool {
    let prod = atomic_producer(q).load(Ordering::Acquire);
    let cons = atomic_conumer(q).load(Ordering::Relaxed);
    prod == cons
}

#[inline]
pub fn queue_full(q: *mut rxe_queue_buf) -> bool {
    let prod = atomic_producer(q).load(Ordering::Relaxed);
    let cons = atomic_conumer(q).load(Ordering::Acquire);
    ((prod + 1) & unsafe { (*q).index_mask }) == cons
}

#[inline]
pub fn advance_producer(q: *mut rxe_queue_buf) {
    let prod = atomic_producer(q).load(Ordering::Relaxed);
    let prod_val = ((prod + 1) & unsafe { (*q).index_mask });
    atomic_producer(q).store(prod_val, Ordering::Release)
}

#[inline]
pub fn advance_consumer(q: *mut rxe_queue_buf) {
    let cons = atomic_conumer(q).load(Ordering::Relaxed);
    let cons_val = ((cons + 1) & unsafe { (*q).index_mask });
    atomic_conumer(q).store(cons_val, Ordering::Release)
}

#[inline]
pub fn load_producer_index(q: *mut rxe_queue_buf) -> u32 {
    atomic_producer(q).load(Ordering::Relaxed)
}

#[inline]
pub fn store_producer_index(q: *mut rxe_queue_buf, index: u32) {
    atomic_producer(q).store(index, Ordering::Release);
}

#[inline]
pub fn load_consumer_index(q: *mut rxe_queue_buf) -> u32 {
    atomic_conumer(q).load(Ordering::Relaxed)
}

#[inline]
pub fn store_consumer_index(q: *mut rxe_queue_buf, index: u32) {
    atomic_conumer(q).store(index, Ordering::Release);
}

#[inline]
pub fn producer_addr<T>(q: *mut rxe_queue_buf) -> *mut T {
    let prod = atomic_producer(q).load(Ordering::Relaxed);
    unsafe {
        ((*q)
            .data
            .as_mut_ptr()
            .add((prod << (*q).log2_elem_size) as usize)) as *mut T
    }
}

#[inline]
pub fn consumer_addr<T>(q: *mut rxe_queue_buf) -> *mut T {
    let cons = atomic_conumer(q).load(Ordering::Relaxed);
    unsafe {
        ((*q)
            .data
            .as_mut_ptr()
            .add((cons << (*q).log2_elem_size) as usize)) as *mut T
    }
}

#[inline]
pub fn addr_from_index<T>(q: *mut rxe_queue_buf, index: u32) -> *mut T {
    let _index = index & unsafe { (*q).index_mask };
    unsafe {
        ((*q)
            .data
            .as_mut_ptr()
            .add((_index << (*q).log2_elem_size) as usize)) as *mut T
    }
}

#[inline]
pub fn index_from_addr<T>(q: *const rxe_queue_buf, addr: *mut T) -> u32 {
    unsafe {
        ((((addr as *mut u8).sub((*q).data.as_ptr() as usize) as u32) >> (*q).log2_elem_size)
            & (*q).index_mask)
    }
}

#[inline]
pub fn advance_cq_cur_index(cq: *mut rxe_cq) {
    let cq = unsafe { &mut *cq };
    let q = unsafe { &*(cq.queue) };
    cq.cur_index = (cq.cur_index + 1) & q.index_mask;
}

#[inline]
pub fn check_cq_queue_empty(cq: *mut rxe_cq) -> bool {
    let cq = unsafe { &*cq };
    let q = unsafe { cq.queue };
    let prod = atomic_producer(q).load(Ordering::Acquire);
    cq.cur_index == prod
}

#[inline]
pub fn advance_qp_cur_index(qp: &mut rxe_qp) {
    let q = unsafe { &*(qp.sq.queue) };
    qp.cur_index = (qp.cur_index + 1) & q.index_mask;
}

#[inline]
pub fn check_qp_queue_full(qp: &mut rxe_qp) -> i32 {
    let mut q = unsafe { qp.sq.queue };
    let cons = atomic_conumer(q).load(Ordering::Acquire);
    if qp.err != 0 {
        return qp.err;
    }
    if cons == ((qp.cur_index + 1) & unsafe { (*q).index_mask }) {
        qp.err = libc::ENOSPC;
    }
    return qp.err;
}

#[cfg(test)]
mod tests {
    use super::*;
    fn new_rxe_queue() -> rxe_queue_buf {
        rxe_queue_buf {
            log2_elem_size: 0xA,
            index_mask: 0xFFFF,
            pad_1: [0; 30],
            producer_index: 0xCCCC,
            pad_2: [0; 31],
            consumer_index: 0xDDDD,
            pad_3: [0; 31],
            data: Default::default(),
        }
    }
    #[test]
    fn check_atomic_from_mut() {
        let mut some_int = 123;
        let a = atomicu32_from_mut(&mut some_int);
        a.store(100, Ordering::Relaxed);
        assert_eq!(some_int, 100);
    }
    #[test]
    fn check_queue_empty() {
        let mut q = &mut new_rxe_queue();
        store_consumer_index(q, 0xFFFF);
        store_producer_index(q, 0xFFFF);
        assert!(queue_empty(q));
    }
    #[test]
    fn check_advance_producer() {
        let mut q = &mut new_rxe_queue();
        advance_producer(q);
        assert_eq!(load_producer_index(q), 0xCCCC + 1);
    }
    #[test]
    fn check_producer_addr() {
        let mut q = &mut new_rxe_queue();
        let mut data: __IncompleteArrayField<u8> = Default::default();
        let data_ptr = data.as_mut_ptr() as usize;
        q.data = data;
        assert_eq!(
            producer_addr::<rxe_send_wqe>(q) as usize,
            data_ptr + (q.producer_index << q.log2_elem_size) as usize
        );
    }
}
