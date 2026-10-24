//! Minimal pipe-layer probe (diagnostic; run with --nocapture).
#![cfg(windows)]

use ramshaper_windows::pipe::{
    client_connect, connect_cancellable, create_instance, read_frame, write_frame,
};

#[test]
fn probe_pipe_roundtrip_step_by_step() {
    let name = format!(r"\\.\pipe\ramshaper-probe-{}", std::process::id());

    let sid = ramshaper_windows::token::current_user_sid().expect("sid");
    eprintln!("[probe] sid ok");
    let sd = ramshaper_windows::token::security_descriptor_for_user(&sid).expect("sd");
    eprintln!("[probe] sd {} bytes", sd.len());

    let instance = create_instance(&name, &sd, true, 4).expect("instance");
    eprintln!("[probe] instance created");

    let client_name = name.clone();
    let client = std::thread::spawn(move || {
        eprintln!("[client] connecting");
        let client = client_connect(&client_name).expect("client connect");
        eprintln!("[client] connected; writing");
        write_frame(&client, b"{\"x\":1}").expect("client write");
        eprintln!("[client] wrote; reading");
        let frame = read_frame(&client).expect("client read");
        eprintln!("[client] read {} bytes", frame.len());
    });

    std::thread::sleep(std::time::Duration::from_millis(300));
    eprintln!("[probe] calling connect_cancellable");
    let connected = connect_cancellable(&instance).expect("connect result");
    eprintln!("[probe] connected={connected}");
    assert!(connected);

    let frame = read_frame(&instance).expect("server read");
    eprintln!("[probe] server read {} bytes", frame.len());
    write_frame(&instance, b"{\"ok\":true}").expect("server write");
    eprintln!("[probe] server wrote");

    client.join().expect("client join");
    eprintln!("[probe] done");
}
