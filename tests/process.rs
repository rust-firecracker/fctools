use std::time::Duration;

use bytes::Bytes;
use fctools::vmm::{
    process::{HyperResponseExt, VmmProcessState},
    resource::MovedVmmResource,
};
use futures_util::{io::BufReader, AsyncBufReadExt, StreamExt};
use http::Uri;
use http_body_util::Full;
use hyper::Request;
use hyper_client_sockets::unix::{HyperUnixStream, UnixUriExt};
use test_framework::{get_resource_references_for_vec, run_vmm_process_test, TestOptions, TestVmmProcess};

mod test_framework;

#[tokio::test]
async fn vmm_can_recv_ctrl_alt_del() {
    run_vmm_process_test(false, |mut process, resources| async move {
        shutdown(&mut process, resources).await;
    })
    .await;
}

#[tokio::test]
async fn vmm_can_recv_sigkill() {
    run_vmm_process_test(true, |mut process, mut resources| async move {
        process.send_sigkill().unwrap();
        process.wait_for_exit().await.unwrap();
        if let VmmProcessState::Crashed(exit_status) = process.state() {
            assert!(!exit_status.success());
        } else {
            panic!("State was not reported as crashed!");
        }
        process
            .cleanup(get_resource_references_for_vec(&mut resources))
            .await
            .unwrap();
        process.send_sigkill().unwrap_err();
    })
    .await;
}

#[tokio::test]
async fn vmm_can_take_out_pipes() {
    run_vmm_process_test(true, |mut process, mut resources| async move {
        let pipes = process.take_pipes().unwrap();
        process.take_pipes().unwrap_err(); // cannot take out pipes twice
        process.send_ctrl_alt_del().await.unwrap();
        assert!(process.wait_for_exit().await.unwrap().success());

        let mut buf = String::new();
        let mut buf_reader = BufReader::new(pipes.stdout).lines();
        while let Some(Ok(line)) = buf_reader.next().await {
            buf.push_str(&line);
            buf.push('\n');
        }
        assert!(buf.contains("Artificially kick devices."));

        process
            .cleanup(get_resource_references_for_vec(&mut resources))
            .await
            .unwrap();
    })
    .await;
}

#[tokio::test]
async fn vmm_operations_are_rejected_in_incorrect_states() {
    run_vmm_process_test(false, |mut process, mut resources| async move {
        process
            .prepare(get_resource_references_for_vec(&mut resources))
            .await
            .unwrap_err();
        process.invoke(None).await.unwrap_err();
        process
            .cleanup(get_resource_references_for_vec(&mut resources))
            .await
            .unwrap_err();

        shutdown(&mut process, resources.clone()).await;

        process.send_sigkill().unwrap_err();
        process.send_ctrl_alt_del().await.unwrap_err();
        process.wait_for_exit().await.unwrap_err();
        process
            .prepare(get_resource_references_for_vec(&mut resources))
            .await
            .unwrap_err();
        process.invoke(None).await.unwrap_err();
    })
    .await;
}

#[tokio::test]
async fn vmm_can_send_get_request_to_api_socket() {
    run_vmm_process_test(false, |mut process, resources| async move {
        let request = Request::builder().method("GET").body(Full::new(Bytes::new())).unwrap();
        let mut response = process.send_api_request("/", request).await.unwrap();
        assert!(response.status().is_success());
        let body = response.read_body_to_string().await.unwrap();
        assert!(body.contains("\"state\":\"Running\""));
        assert!(body.contains("\"vmm_version\":\""));
        assert!(body.contains("\"app_name\":\"Firecracker\""));
        shutdown(&mut process, resources).await;
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
        assert!(response.read_body_to_string().await.unwrap().is_empty());
    }

    run_vmm_process_test(false, |mut process, resources| async move {
        send_state_request("Paused", &mut process).await;
        send_state_request("Resumed", &mut process).await;
        shutdown(&mut process, resources).await;
    })
    .await;
}

#[tokio::test]
async fn vmm_can_send_put_request_to_api_socket() {
    run_vmm_process_test(false, |mut process, resources| async move {
        let request = Request::builder()
            .method("PUT")
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from("{\"action_type\":\"FlushMetrics\"}")))
            .unwrap();
        let mut response = process.send_api_request("/actions", request).await.unwrap();
        assert!(response.status().is_success());
        assert!(response.read_body_to_string().await.unwrap().is_empty());
        shutdown(&mut process, resources).await;
    })
    .await;
}

#[tokio::test]
async fn vmm_get_socket_path_returns_correct_path() {
    run_vmm_process_test(false, |mut process, resources| async move {
        let socket_path = process.get_socket_path().unwrap();
        process
            .send_api_request(
                "/",
                Request::builder().method("GET").body(Full::new(Bytes::new())).unwrap(),
            )
            .await
            .unwrap();

        let (mut send_request, connection) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(
            HyperUnixStream::connect(&socket_path, hyper_client_sockets::Backend::Tokio)
                .await
                .unwrap(),
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
        shutdown(&mut process, resources).await;
    })
    .await;
}

#[tokio::test]
async fn vmm_inner_to_outer_path_performs_transformation() {
    run_vmm_process_test(false, |mut process, resources| async move {
        let effective_path = process.local_to_effective_path("/dev/kvm");

        if effective_path.to_str().unwrap() != "/dev/kvm"
            && !effective_path.starts_with("/srv/jailer")
            && effective_path.ends_with("/root/dev/kvm")
        {
            panic!("Expected outer path transformation to succeed, instead received: {effective_path:?}");
        }

        shutdown(&mut process, resources).await;
    })
    .await;
}

async fn shutdown(process: &mut TestVmmProcess, mut resources: Vec<MovedVmmResource>) {
    if tokio::time::timeout(
        Duration::from_millis(TestOptions::get().await.waits.shutdown_timeout_ms),
        async {
            process.send_ctrl_alt_del().await.unwrap();
            process.wait_for_exit().await.unwrap();
        },
    )
    .await
    .is_err()
    {
        process.send_sigkill().unwrap();
        process.wait_for_exit().await.unwrap();
    }

    process
        .cleanup(get_resource_references_for_vec(&mut resources))
        .await
        .unwrap();
}
