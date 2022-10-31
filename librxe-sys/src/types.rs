use crate::*;
use std::os::raw::{c_int, c_uint};

// rxe_cq related union and struct types
#[repr(C)]
#[derive(Clone, Copy)]
pub union verbs_cq_union {
    pub cq: rdma_sys::ibv_cq,
    pub cq_ex: rdma_sys::ibv_cq_ex,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct verbs_cq {
    pub cq_union: verbs_cq_union,
}
#[repr(C)]
pub struct rxe_cq {
    pub vcq: verbs_cq,
    pub mmap_info: mminfo,
    pub queue: *mut rxe_queue_buf,
    pub lock: libc::pthread_spinlock_t,
    pub wc: *mut ib_uverbs_wc,
    pub wc_size: usize,
    pub cur_index: u32,
}

#[repr(C)]
pub struct rxe_ah {
    pub ibv_ah: rdma_sys::ibv_ah,
    pub av: rxe_av,
    pub ah_num: c_int,
}
// rxe_wq related union and struct types
#[repr(C)]
pub struct rxe_wq {
    pub queue: *mut rxe_queue_buf,
    pub lock: libc::pthread_spinlock_t,
    pub max_sge: c_uint,
    pub max_inline: c_uint,
}

// rxe_qp related union and struct types
#[repr(C)]
#[derive(Clone, Copy)]
pub union verbs_qp_union_t {
    pub qp: rdma_sys::ibv_qp,
    pub qp_ex: rdma_sys::ibv_qp_ex,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct verbs_xrcd {
    xrcd: rdma_sys::ibv_xrcd,
    comp_mask: u32,
    handler: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct verbs_qp {
    pub qp_union: verbs_qp_union_t,
    pub comp_mask: u32,
    pub xrcd: *mut verbs_xrcd,
}

#[repr(C)]
pub struct rxe_qp {
    pub vqp: verbs_qp,
    pub rq_mmap_info: mminfo,
    pub rq: rxe_wq,
    pub sq_mmap_info: mminfo,
    pub sq: rxe_wq,
    pub ssn: c_uint,
    pub cur_index: u32,
    pub err: c_int,
}
