use std::time::Duration;

use bytes::Bytes;
use fctools::{
    executor::arguments::FirecrackerConfigOverride,
    process::{HyperResponseExt, VmmProcessState},
};
use http::Uri;
use http_body_util::Full;
use hyper::Request;
use hyper_client_sockets::{HyperUnixStream, UnixUriExt};
use test_framework::{run_vmm_process_test, TestVmmProcess};
use tokio::io::{AsyncBufReadExt, BufReader};

mod test_framework;

#[tokio::test]
async fn vmm_can_recv_ctrl_alt_del() {
    run_vmm_process_test(|mut process| async move {
        shutdown(&mut process).await;
        assert_eq!(process.state(), VmmProcessState::Exited);
    })
    .await;
}

#[tokio::test]
async fn vmm_can_recv_sigkill() {
    run_vmm_process_test(|mut process| async move {
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
    run_vmm_process_test(|mut process| async move {
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
    run_vmm_process_test(|mut process| async move {
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
    run_vmm_process_test(|mut process| async move {
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
async fn vmm_can_send_put_and_patch_requests_to_api_socket() {
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

    run_vmm_process_test(|mut process| async move {
        send_state_request("Paused", &mut process).await;
        send_state_request("Resumed", &mut process).await;
        shutdown(&mut process).await;
    })
    .await;
    run_vmm_process_test(|mut process| async move {
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
    run_vmm_process_test(|mut process| async move {
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
    run_vmm_process_test(|mut process| async move {
        let outer_path = process.inner_to_outer_path("/dev/kvm");

        if outer_path.to_str().unwrap() != "/dev/kvm"
            && !outer_path.starts_with("/srv/jailer")
            && outer_path.ends_with("/root/dev/kvm")
        {
            panic!("Expected outer path transformation to succeed, instead received: {outer_path:?}");
        }

        shutdown(&mut process).await;
    })
    .await;
}

async fn shutdown(process: &mut TestVmmProcess) {
    if tokio::time::timeout(Duration::from_millis(1000), async {
        process.send_ctrl_alt_del().await.unwrap();
    })
    .await
    .is_err()
    {
        process.send_sigkill().unwrap();
    }

    assert!(process.wait_for_exit().await.unwrap().success());
    process.cleanup().await.unwrap();
}
