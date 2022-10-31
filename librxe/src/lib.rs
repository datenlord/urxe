use nix::Error;
use rdma_sys::{ibv_qp_state, ibv_qp_type, ibv_send_flags, ibv_wr_opcode};

pub fn rxe_post_send(
    ibqp: *mut rdma_sys::ibv_qp,
    wr_list: *mut rdma_sys::ibv_send_wr,
    bad_wr: *mut *mut rdma_sys::ibv_send_wr,
) -> Result<(), Error> {
    if ibqp.is_null() || bad_wr.is_null() || wr_list.is_null() {
        return Err(Error::EINVAL);
    }
    let rqp = librxe_sys::to_rqp(ibqp).expect("unable to find rxe_qp from ib qp");

    unsafe {
        *bad_wr = std::ptr::null_mut();
        libc::pthread_spin_lock(&mut (*rqp).sq.lock);
    }
    let mut rc_errno = Error::UnknownErrno;
    let mut wr_list = wr_list;
    while !wr_list.is_null() {
        if let Err(e) = post_one_send(rqp, unsafe { &mut (*rqp).sq }, wr_list) {
            unsafe {
                *bad_wr = wr_list;
            }
            rc_errno = e;
            break;
        }
        wr_list = unsafe { (*wr_list).next };
    }
    unsafe {
        libc::pthread_spin_unlock(&mut (*rqp).sq.lock);
    }
    let ibqp = unsafe { &mut *ibqp };

    if let Err(e) = post_send_db(ibqp) {
        return Err(e);
    } else if rc_errno != Error::UnknownErrno {
        return Err(rc_errno);
    } else {
        return Ok(());
    }
}

pub fn rxe_post_recv(
    ibqp: *mut rdma_sys::ibv_qp,
    recv_wr: *mut rdma_sys::ibv_recv_wr,
    bad_wr: *mut *mut rdma_sys::ibv_recv_wr,
) -> Result<(), Error> {
    if bad_wr.is_null() || recv_wr.is_null() {
        return Err(Error::EINVAL);
    }
    let rqp = librxe_sys::to_rqp(ibqp).expect("unable to find rxe_qp from ib qp");
    let ibqp = unsafe { &mut *ibqp };
    if ibqp.state == ibv_qp_state::IBV_QPS_RESET {
        return Err(Error::EINVAL);
    }
    unsafe {
        *bad_wr = std::ptr::null_mut();
        libc::pthread_spin_lock(&mut (*rqp).rq.lock);
    }
    let mut rc_errno = Error::UnknownErrno;
    let mut recv_wr = recv_wr;
    while !recv_wr.is_null() {
        if let Err(e) = rxe_post_one_recv(unsafe { &mut (*rqp).rq }, recv_wr) {
            rc_errno = e;
            unsafe {
                *bad_wr = recv_wr;
            }
            break;
        }
        recv_wr = unsafe { (*recv_wr).next };
    }
    unsafe {
        libc::pthread_spin_unlock(&mut (*rqp).rq.lock);
    }

    if rc_errno != Error::UnknownErrno {
        return Err(rc_errno);
    } else {
        return Ok(());
    }
}

pub fn rxe_poll_cq(
    ibcq: *mut rdma_sys::ibv_cq,
    ne: libc::c_int,
    mut wc: *mut rdma_sys::ibv_wc,
) -> libc::c_int {
    let cq = librxe_sys::to_rcq(ibcq).expect("unable to find rxe_cq from ibv_cq");
    unsafe {
        libc::pthread_spin_lock(&mut (*cq).lock);
    }
    let q = unsafe { (*cq).queue };
    let mut npolled = 0;
    while npolled < ne {
        if librxe_sys::queue_empty(q) {
            break;
        }
        let src_addr: *mut rdma_sys::ibv_wc = librxe_sys::consumer_addr(q);
        unsafe {
            std::ptr::copy_nonoverlapping(src_addr, wc, 1);
        }
        librxe_sys::advance_consumer(q);
        npolled = npolled + 1;
        unsafe { wc = wc.add(1) };
    }
    unsafe {
        libc::pthread_spin_unlock(&mut (*cq).lock);
    }

    return npolled;
}

fn post_one_send(
    qp: *mut librxe_sys::rxe_qp,
    sq: *mut librxe_sys::rxe_wq,
    ibwr: *mut rdma_sys::ibv_send_wr,
) -> Result<(), Error> {
    let mut length = 0;
    let qp = unsafe { &mut *qp };
    let sq = unsafe { &mut *sq };
    let ibwr = unsafe { &mut *ibwr };
    for i in 0..ibwr.num_sge {
        length += unsafe { (*(ibwr.sg_list.add(i as usize))).length }
    }
    if let Err(e) = validate_send_wr(qp, ibwr, length) {
        return Err(e);
    }
    let wqe = unsafe { &mut *(librxe_sys::producer_addr::<librxe_sys::rxe_send_wqe>(sq.queue)) };
    if let Err(e) = init_send_wqe(qp, sq, ibwr, length, wqe) {
        return Err(e);
    }
    if librxe_sys::queue_full(sq.queue) {
        return Err(Error::ENOMEM);
    }
    librxe_sys::advance_producer(sq.queue);
    Ok(())
}

fn validate_send_wr(
    qp: &librxe_sys::rxe_qp,
    ibwr: &rdma_sys::ibv_send_wr,
    length: u32,
) -> Result<(), Error> {
    let sq = &qp.sq;
    let opcode = ibwr.opcode;
    if ibwr.num_sge as u32 > sq.max_sge {
        return Err(Error::EINVAL);
    }
    if (opcode == ibv_wr_opcode::IBV_WR_ATOMIC_CMP_AND_SWP)
        || (opcode == ibv_wr_opcode::IBV_WR_ATOMIC_FETCH_AND_ADD)
    {
        if length < 8 || unsafe { (ibwr.wr.atomic.remote_addr & 0x7) != 0 } {
            return Err(Error::EINVAL);
        }
    }
    if (ibwr.send_flags & ibv_send_flags::IBV_SEND_INLINE.0) != 0 && (length > sq.max_inline) {
        return Err(Error::EINVAL);
    }
    if ibwr.opcode == ibv_wr_opcode::IBV_WR_BIND_MW {
        if length != 0 {
            return Err(Error::EINVAL);
        }
        if ibwr.num_sge != 0 {
            return Err(Error::EINVAL);
        }
        if unsafe { ibwr.imm_data_invalidated_rkey_union.imm_data } != 0 {
            return Err(Error::EINVAL);
        }
        if (librxe_sys::qp_type(&*qp) != ibv_qp_type::IBV_QPT_RC)
            && (librxe_sys::qp_type(&*qp) != ibv_qp_type::IBV_QPT_UC)
        {
            return Err(Error::EINVAL);
        }
    }
    Ok(())
}

fn convert_send_wr(
    qp: &librxe_sys::rxe_qp,
    kwr: &mut librxe_sys::rxe_send_wr,
    uwr: &rdma_sys::ibv_send_wr,
) {
    unsafe {
        *kwr = std::mem::zeroed::<librxe_sys::rxe_send_wr>();
    }
    kwr.wr_id = uwr.wr_id;
    kwr.num_sge = uwr.num_sge as u32;
    kwr.opcode = uwr.opcode;
    kwr.send_flags = uwr.send_flags;
    kwr.ex.imm_data = unsafe { uwr.imm_data_invalidated_rkey_union.imm_data };
    match uwr.opcode {
        ibv_wr_opcode::IBV_WR_RDMA_WRITE
        | ibv_wr_opcode::IBV_WR_RDMA_WRITE_WITH_IMM
        | ibv_wr_opcode::IBV_WR_RDMA_READ => unsafe {
            kwr.wr.rdma.as_mut().remote_addr = uwr.wr.rdma.remote_addr;
            kwr.wr.rdma.as_mut().rkey = uwr.wr.rdma.rkey;
        },
        ibv_wr_opcode::IBV_WR_SEND | ibv_wr_opcode::IBV_WR_SEND_WITH_IMM => {
            if librxe_sys::qp_type(qp) == ibv_qp_type::IBV_QPT_UD {
                unsafe {
                    let ah = &*librxe_sys::to_rah(uwr.wr.ud.ah)
                        .expect("unable to find rxe_ah from ib_ah");
                    kwr.wr.ud.as_mut().remote_qpn = uwr.wr.ud.remote_qpn;
                    kwr.wr.ud.as_mut().remote_qkey = uwr.wr.ud.remote_qkey;
                    kwr.wr.ud.as_mut().ah_num = ah.ah_num as _;
                }
            }
        }
        ibv_wr_opcode::IBV_WR_ATOMIC_CMP_AND_SWP | ibv_wr_opcode::IBV_WR_ATOMIC_FETCH_AND_ADD => unsafe {
            kwr.wr.atomic.as_mut().remote_addr = uwr.wr.atomic.remote_addr;
            kwr.wr.atomic.as_mut().compare_add = uwr.wr.atomic.compare_add;
            kwr.wr.atomic.as_mut().swap = uwr.wr.atomic.swap;
            kwr.wr.atomic.as_mut().rkey = uwr.wr.atomic.rkey;
        },
        ibv_wr_opcode::IBV_WR_BIND_MW => unsafe {
            let ibmr = *uwr.bind_mw_tso_union.bind_mw.bind_info.mr;
            let ibmw = *uwr.bind_mw_tso_union.bind_mw.mw;
            kwr.wr.mw.as_mut().addr = uwr.bind_mw_tso_union.bind_mw.bind_info.addr;
            kwr.wr.mw.as_mut().length = uwr.bind_mw_tso_union.bind_mw.bind_info.length;
            kwr.wr.mw.as_mut().mr_lkey = ibmr.lkey;
            kwr.wr.mw.as_mut().mw_rkey = ibmw.rkey;
            kwr.wr.mw.as_mut().rkey = uwr.bind_mw_tso_union.bind_mw.rkey;
            kwr.wr.mw.as_mut().access = uwr.bind_mw_tso_union.bind_mw.bind_info.mw_access_flags;
        },
        _ => {}
    }
}

fn init_send_wqe(
    qp: &mut librxe_sys::rxe_qp,
    _sq: &librxe_sys::rxe_wq,
    ibwr: &rdma_sys::ibv_send_wr,
    length: u32,
    wqe: &mut librxe_sys::rxe_send_wqe,
) -> Result<(), Error> {
    let num_sge = ibwr.num_sge as u32;
    let opcode = ibwr.opcode;
    convert_send_wr(qp, &mut wqe.wr, ibwr);
    if librxe_sys::qp_type(qp) == ibv_qp_type::IBV_QPT_UD {
        let ah = unsafe {
            &*(librxe_sys::to_rah(ibwr.wr.ud.ah).expect("Unable to find rxe_ah from ib_ah"))
        };
        if ah.ah_num == 0 {
            unsafe {
                std::ptr::copy_nonoverlapping::<librxe_sys::rxe_av>(
                    &ah.av,
                    &mut wqe.wr.wr.ud.as_mut().av,
                    1,
                )
            }
        }
    }
    if (ibwr.send_flags & ibv_send_flags::IBV_SEND_INLINE.0) != 0 {
        unsafe {
            let mut inline_data = wqe
                .dma
                .__bindgen_anon_1
                .__bindgen_anon_1
                .as_mut()
                .inline_data
                .as_mut_ptr();
            for i in 0..num_sge as usize {
                let cur_length = (*(ibwr.sg_list.add(i))).length as usize;
                std::ptr::copy_nonoverlapping(
                    (*(ibwr.sg_list.add(i))).addr as *mut u8,
                    inline_data,
                    cur_length,
                );
                inline_data = inline_data.add(cur_length);
            }
        }
    } else {
        unsafe {
            std::ptr::copy_nonoverlapping::<rdma_sys::ibv_sge>(
                ibwr.sg_list,
                wqe.dma
                    .__bindgen_anon_1
                    .__bindgen_anon_2
                    .as_mut()
                    .sge
                    .as_mut_ptr() as *mut rdma_sys::ibv_sge,
                num_sge as usize,
            )
        }
    }
    if (opcode == ibv_wr_opcode::IBV_WR_ATOMIC_CMP_AND_SWP)
        || (opcode == ibv_wr_opcode::IBV_WR_ATOMIC_FETCH_AND_ADD)
    {
        wqe.iova = unsafe { ibwr.wr.atomic.remote_addr };
    } else {
        wqe.iova = unsafe { ibwr.wr.rdma.remote_addr };
    }
    wqe.dma.length = length;
    wqe.dma.resid = length;
    wqe.dma.num_sge = num_sge;
    wqe.dma.cur_sge = 0;
    wqe.dma.sge_offset = 0;
    wqe.state = 0;
    qp.ssn = qp.ssn + 1;
    wqe.ssn = qp.ssn;

    Ok(())
}

fn post_send_db(ibqp: &mut rdma_sys::ibv_qp) -> Result<(), Error> {
    let cmd = &mut librxe_sys::ibv_post_send::default();
    let resp = &mut librxe_sys::ib_uverbs_post_send_resp::default();
    cmd.hdr.command = librxe_sys::ib_uverbs_write_cmds::IB_USER_VERBS_CMD_POST_SEND;
    cmd.hdr.in_words = (std::mem::size_of_val(cmd) / 4) as _;
    cmd.hdr.out_words = (std::mem::size_of_val(resp) / 4) as _;
    unsafe {
        cmd.__bindgen_anon_1.__bindgen_anon_1.as_mut().response = resp as *mut _ as u64;
        cmd.__bindgen_anon_1.__bindgen_anon_1.as_mut().qp_handle = ibqp.handle;
        cmd.__bindgen_anon_1.__bindgen_anon_1.as_mut().wr_count = 0;
        cmd.__bindgen_anon_1.__bindgen_anon_1.as_mut().sge_count = 0;
        cmd.__bindgen_anon_1.__bindgen_anon_1.as_mut().wqe_size =
            std::mem::size_of::<rdma_sys::ibv_send_wr>() as _;
    }
    unsafe {
        let cmd_buf = librxe_sys::serialize_raw(cmd);
        let res = nix::unistd::write((*ibqp.context).cmd_fd, cmd_buf).unwrap();
        if res != std::mem::size_of::<librxe_sys::ibv_post_send>() {
            return Err(nix::errno::from_i32(nix::errno::errno()));
        }
    }

    Ok(())
}

fn rxe_post_one_recv(
    rq: *mut librxe_sys::rxe_wq,
    recv_wr: *mut rdma_sys::ibv_recv_wr,
) -> Result<(), Error> {
    let rq = unsafe { &mut *rq };
    let recv_wr = unsafe { &mut *recv_wr };
    let mut length = 0;
    if librxe_sys::queue_full(rq.queue) {
        return Err(Error::ENOMEM);
    }
    if recv_wr.num_sge as u32 > rq.max_sge {
        return Err(Error::EINVAL);
    }
    let wqe = unsafe { &mut *(librxe_sys::producer_addr::<librxe_sys::rxe_recv_wqe>(rq.queue)) };
    wqe.wr_id = recv_wr.wr_id;
    wqe.num_sge = recv_wr.num_sge as _;
    unsafe {
        std::ptr::copy_nonoverlapping::<librxe_sys::rxe_sge>(
            recv_wr.sg_list as *mut librxe_sys::rxe_sge,
            wqe.dma
                .__bindgen_anon_1
                .__bindgen_anon_2
                .as_mut()
                .sge
                .as_mut_ptr(),
            wqe.num_sge as _,
        );
        for i in 0..wqe.num_sge as usize {
            length = length
                + (*wqe
                    .dma
                    .__bindgen_anon_1
                    .__bindgen_anon_2
                    .as_ref()
                    .sge
                    .as_ptr()
                    .add(i))
                .length;
        }
    }
    wqe.dma.length = length;
    wqe.dma.resid = length;
    wqe.dma.cur_sge = 0;
    wqe.dma.num_sge = wqe.num_sge;
    wqe.dma.sge_offset = 0;
    librxe_sys::advance_producer(rq.queue);

    Ok(())
}
