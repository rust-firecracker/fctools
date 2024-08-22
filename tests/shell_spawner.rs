use fctools::shell_spawner::{SameUserShellSpawner, ShellSpawner, SuShellSpawner, SudoShellSpawner};

#[tokio::test]
async fn same_user_shell_launches_simple_command() {
    let shell_spawner = SameUserShellSpawner::new(which::which("sh").unwrap());
    let child = shell_spawner.spawn("cat --help".into()).await.unwrap();
    let output = child.wait_with_output().await.unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(stdout.contains("Usage: cat [OPTION]... [FILE]..."));
    assert!(stdout.contains("or available locally via: info '(coreutils) cat invocation'"))
}

#[tokio::test]
async fn same_user_shell_runs_under_correct_uid() {
    let uid = unsafe { libc::geteuid() };
    let shell_spawner = SameUserShellSpawner::new(which::which("sh").unwrap());
    let stdout = String::from_utf8_lossy(
        &shell_spawner
            .spawn("echo $UID".into())
            .await
            .unwrap()
            .wait_with_output()
            .await
            .unwrap()
            .stdout,
    )
    .into_owned();
    assert_eq!(stdout.trim_end().parse::<u32>().unwrap(), uid);
}

#[tokio::test]
async fn su_shell_should_elevate() {
    elevation_test(SuShellSpawner::new).await;
}

#[tokio::test]
async fn sudo_shell_should_elevate() {
    elevation_test(SudoShellSpawner::with_password).await;
}

async fn elevation_test<F, S>(closure: F)
where
    F: FnOnce(String) -> S,
    S: ShellSpawner,
{
    let password = std::env::var("ROOT_PWD");
    if password.is_err() {
        println!("Test was skipped due to ROOT_PWD not being set");
        return;
    }
    let shell_spawner = closure(password.unwrap());
    let child = shell_spawner.spawn("echo $UID".into()).await.unwrap();
    let stdout = String::from_utf8_lossy(&child.wait_with_output().await.unwrap().stdout).into_owned();
    assert_eq!(stdout, "0\n");
}
