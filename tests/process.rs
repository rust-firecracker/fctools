use std::{future::Future, path::PathBuf, time::Duration};

use bytes::Bytes;
use common::{TestExecutor, TestShellSpawner, TestVmmProcess};
use fctools::{
    executor::{
        arguments::{FirecrackerApiSocket, FirecrackerArguments, FirecrackerConfigOverride, JailerArguments},
        installation::FirecrackerInstallation,
        jailed::{FlatJailRenamer, JailedVmmExecutor},
        unrestricted::UnrestrictedVmmExecutor,
    },
    process::{HyperResponseExt, VmmProcess, VmmProcessState},
    shell_spawner::{SameUserShellSpawner, SuShellSpawner},
};
use http::Uri;
use http_body_util::Full;
use hyper::Request;
use hyper_client_sockets::{HyperUnixStream, UnixUriExt};
use rand::RngCore;
use tokio::io::{AsyncBufReadExt, BufReader};
use uuid::Uuid;

mod common;

#[tokio::test]
async fn vmm_can_recv_ctrl_alt_del() {
    vmm_test(|mut process| async move {
        shutdown(&mut process).await;
        assert_eq!(process.state(), VmmProcessState::Exited);
    })
    .await;
}

#[tokio::test]
async fn vmm_can_recv_sigkill() {
    vmm_test(|mut process| async move {
        process.send_sigkill().unwrap();
        process.wait_for_exit().await.unwrap();
        if let VmmProcessState::Crashed(exit_status) = process.state() {
            assert!(!exit_status.success());
        } else {
            panic!("State was not reported as crashed!");
        }
        process.cleanup().await.unwrap();
        process.send_sigkill().unwrap_err();
    })
    .await;
}

#[tokio::test]
async fn vmm_can_take_out_pipes() {
    vmm_test(|mut process| async move {
        let pipes = process.take_pipes().unwrap();
        process.take_pipes().unwrap_err(); // cannot take out pipes twice
        process.send_ctrl_alt_del().await.unwrap();
        assert!(process.wait_for_exit().await.unwrap().success());

        let mut buf = String::new();
        let mut buf_reader = BufReader::new(pipes.stdout).lines();
        while let Ok(Some(line)) = buf_reader.next_line().await {
            buf.push_str(&line);
            buf.push('\n');
        }
        assert!(buf.contains("Artificially kick devices."));
        assert!(buf.contains("Firecracker exiting successfully. exit_code=0"));

        process.cleanup().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn vmm_operations_are_rejected_in_incorrect_states() {
    vmm_test(|mut process| async move {
        process.prepare().await.unwrap_err();
        process.invoke(FirecrackerConfigOverride::NoOverride).await.unwrap_err();
        process.cleanup().await.unwrap_err();

        shutdown(&mut process).await;

        process.send_sigkill().unwrap_err();
        process.send_ctrl_alt_del().await.unwrap_err();
        process.wait_for_exit().await.unwrap_err();
        process.prepare().await.unwrap_err();
        process.invoke(FirecrackerConfigOverride::NoOverride).await.unwrap_err();
    })
    .await;
}

#[tokio::test]
async fn vmm_can_send_get_request_to_api_socket() {
    vmm_test(|mut process| async move {
        let request = Request::builder().method("GET").body(Full::new(Bytes::new())).unwrap();
        let mut response = process.send_api_request("/", request).await.unwrap();
        assert!(response.status().is_success());
        let body = response.recv_to_string().await.unwrap();
        assert!(body.contains("\"state\":\"Running\""));
        assert!(body.contains("\"vmm_version\":\""));
        assert!(body.contains("\"app_name\":\"Firecracker\""));
        shutdown(&mut process).await;
    })
    .await;
}

#[tokio::test]
async fn vmm_can_send_patch_request_to_api_socket() {
    async fn send_state_request(state: &str, process: &mut TestVmmProcess) {
        let request = Request::builder()
            .method("PATCH")
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from("{\"state\":\"q\"}".replace("q", state))))
            .unwrap();
        let mut response = process.send_api_request("/vm", request).await.unwrap();
        assert!(response.status().is_success());
        assert!(response.recv_to_string().await.unwrap().is_empty());
    }

    vmm_test(|mut process| async move {
        send_state_request("Paused", &mut process).await;
        send_state_request("Resumed", &mut process).await;
        shutdown(&mut process).await;
    })
    .await;
    vmm_test(|mut process| async move {
        let request = Request::builder()
            .method("PUT")
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from("{\"action_type\":\"FlushMetrics\"}")))
            .unwrap();
        let mut response = process.send_api_request("/actions", request).await.unwrap();
        assert!(response.status().is_success());
        assert!(response.recv_to_string().await.unwrap().is_empty());
        shutdown(&mut process).await;
    })
    .await;
}

#[tokio::test]
async fn vmm_get_socket_path_returns_correct_path() {
    vmm_test(|mut process| async move {
        let socket_path = process.get_socket_path().unwrap();
        let (mut send_request, connection) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(
            HyperUnixStream::connect(&socket_path).await.unwrap(),
        )
        .await
        .unwrap();
        tokio::spawn(connection);

        let response = send_request
            .send_request(
                Request::builder()
                    .method("GET")
                    .uri(Uri::unix(socket_path, "/actions").unwrap())
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(response.status().is_success());
        shutdown(&mut process).await;
    })
    .await;
}

#[tokio::test]
async fn vmm_inner_to_outer_path_performs_transformation() {
    vmm_test(|process| async move {
        let outer_path = process.inner_to_outer_path("/dev/kvm");

        if outer_path.to_str().unwrap() != "/dev/kvm"
            && !outer_path.starts_with("/srv/jailer")
            && outer_path.ends_with("/root/dev/kvm")
        {
            panic!("Expected outer path transformation to succeed, instead received: {outer_path:?}");
        }
    })
    .await;
}

async fn shutdown(handle: &mut TestVmmProcess) {
    handle.send_ctrl_alt_del().await.unwrap();
    assert!(handle.wait_for_exit().await.unwrap().success());
    handle.cleanup().await.unwrap();
}

async fn vmm_test<F, Fut>(closure: F)
where
    F: Fn(TestVmmProcess) -> Fut,
    F: 'static,
    Fut: Future<Output = ()>,
{
    async fn init_process(process: &mut TestVmmProcess) {
        process.wait_for_exit().await.unwrap_err();
        process.send_ctrl_alt_del().await.unwrap_err();
        process.send_sigkill().unwrap_err();
        process.take_pipes().unwrap_err();
        process.cleanup().await.unwrap_err();

        assert_eq!(process.state(), VmmProcessState::AwaitingPrepare);
        process.prepare().await.unwrap();
        assert_eq!(process.state(), VmmProcessState::AwaitingStart);
        process.invoke(FirecrackerConfigOverride::NoOverride).await.unwrap();
        assert_eq!(process.state(), VmmProcessState::Started);
    }

    let (mut unrestricted_process, mut jailed_process) = get_vmm_processes();

    init_process(&mut jailed_process).await;
    init_process(&mut unrestricted_process).await;
    tokio::time::sleep(Duration::from_millis(2000)).await;
    closure(unrestricted_process).await;
    println!("Succeeded with unrestricted VM");
    closure(jailed_process).await;
    println!("Succeeded with jailed VM");
}

fn get_vmm_processes() -> (TestVmmProcess, TestVmmProcess) {
    let socket_path: PathBuf = format!("/tmp/{}", Uuid::new_v4()).into();
    let env_paths = common::get_environment_paths();

    let unrestricted_firecracker_arguments =
        FirecrackerArguments::new(FirecrackerApiSocket::Enabled(socket_path.clone())).config_path(&env_paths.config);
    let jailer_firecracker_arguments =
        FirecrackerArguments::new(FirecrackerApiSocket::Enabled(socket_path)).config_path("jail-config.json");

    let jailer_arguments = JailerArguments::new(
        unsafe { libc::geteuid() },
        unsafe { libc::getegid() },
        rand::thread_rng().next_u32().to_string(),
    );
    let unrestricted_executor = UnrestrictedVmmExecutor::new(unrestricted_firecracker_arguments);
    let jailed_executor = JailedVmmExecutor::new(
        jailer_firecracker_arguments,
        jailer_arguments,
        FlatJailRenamer::default(),
    );
    let su_shell_spawner = SuShellSpawner::new(std::env::var("ROOT_PWD").expect("No ROOT_PWD set"));
    let same_user_shell_spawner = SameUserShellSpawner {
        shell_path: PathBuf::from("/usr/bin/bash"),
    };
    let installation = FirecrackerInstallation {
        firecracker_path: env_paths.firecracker,
        jailer_path: env_paths.jailer,
        snapshot_editor_path: env_paths.snapshot_editor,
    };

    (
        VmmProcess::new(
            TestExecutor::Unrestricted(unrestricted_executor),
            TestShellSpawner::SameUser(same_user_shell_spawner),
            installation.clone(),
            vec![env_paths.kernel.clone(), env_paths.rootfs.clone(), env_paths.config],
        ),
        VmmProcess::new(
            TestExecutor::Jailed(jailed_executor),
            TestShellSpawner::Su(su_shell_spawner),
            installation,
            vec![env_paths.kernel, env_paths.rootfs, env_paths.jail_config],
        ),
    )
}
