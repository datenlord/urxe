use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:include=vendor/rdma-core/build/include");
    println!("cargo:rustc-link-search=native=vendor/rdma-core/build/lib");
    println!("cargo:rustc-link-lib=ibverbs");

    // initialize and update submodules
    if Path::new(".git").is_dir() {
        Command::new("git")
            .args(&["submodule", "update", "--init"])
            .status()
            .expect("Failed to update submodules.");
    } else {
        assert!(
            Path::new("vendor/rdma-core").is_dir(),
            "vendor source not included"
        );
    }

    // build vendor/rdma-core
    Command::new("bash")
        .current_dir("vendor/rdma-core/")
        .args(&["build.sh"])
        .status()
        .expect("Failed to build vendor/rdma-core using build.sh");

    // generate the bindings
    let bindings = bindgen::Builder::default()
        .header("vendor/rdma-core/providers/rxe/rxe-abi.h")
        .header("vendor/rdma-core/kernel-headers/rdma/rdma_user_rxe.h")
        .header("vendor/rdma-core/kernel-headers/rdma/ib_user_verbs.h")
        .header("vendor/rdma-core/libibverbs/kern-abi.h")
        .clang_arg("-Ivendor/rdma-core/build/include/")
        .blocklist_type("in6_addr")
        .blocklist_type("pthread_.*")
        .blocklist_type("sockaddr.*")
        .blocklist_type("timespec")
        .allowlist_type("mminfo")
        .allowlist_type("rxe_queue_buf")
        .allowlist_type("ib_uverbs_wc")
        .allowlist_type("ibv_post_send")
        .allowlist_type("ib_uverbs_post_send_resp")
        .allowlist_type("ib_uverbs_write_cmds")
        .allowlist_type("rxe_send_wqe")
        .allowlist_type("rxe_recv_wqe")
        .constified_enum_module("ib_uverbs_write_cmds")
        .derive_copy(true)
        .derive_default(true)
        .derive_debug(true)
        .prepend_enum_name(false)
        .size_t_is_usize(true)
        .generate()
        .expect("Unable to generate bindings");

    // write the bindings to the $OUT_DIR/bindings.rs file.
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Could not write bindings");
}
