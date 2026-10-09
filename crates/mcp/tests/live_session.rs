//! A separate client discovers the real loopback server and edits the live scene.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use genos_mcp::{
    discovery_dir, endpoint_at, live_endpoints, parse, publish_lighting, Host, Server, Value,
};
use genos_physics::Shape as Collider;
use genos_render::World;
use genos_scene::{look_direction, update, Actions, Camera, Scene, Vec3, CAMERA_HEIGHT};

fn shipped_scene() -> Scene {
    genos_scene::load_path(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/camera/scene.rhai"),
    )
    .expect("shipped scene")
}

#[test]
fn a_separate_client_discovers_reads_and_edits_the_live_scene() {
    refuse_a_dead_process();

    let placed = shipped_scene();
    let mut camera = Camera::opening();
    camera.attach_scene(&placed);
    let host = Host::new(placed.clone(), camera);
    let server = Server::start(host.clone()).expect("server");
    let url = server.url().to_string();
    assert!(url.starts_with("http://127.0.0.1:"), "{url}");
    assert!(url.ends_with("/mcp"), "{url}");

    let record = discovery_dir().join(format!("{}.json", std::process::id()));
    let found = endpoint_at(&record).expect("live discovery record");
    assert_eq!(found.pid, std::process::id());
    assert_eq!(found.url, url);
    assert!(
        live_endpoints()
            .iter()
            .any(|item| item.pid == found.pid && item.url == url),
        "discovery missed this process"
    );

    let mut client = Client::new(&url);
    let opened = client.initialize();
    assert!(opened.contains("\"subscribe\":true"), "{opened}");
    assert!(opened.contains("\"genos\""), "{opened}");
    client.notify_initialized();

    let refused = client.post_origin(
        2,
        "tools/call",
        r#"{"name":"set_object","arguments":{"handle":"floor","position":[9,0,9]}}"#,
        "http://127.0.0.1.evil.com",
    );
    assert_eq!(refused.status, 403, "{}", refused.body);
    assert!(
        (host.drawn_scene().floor.position.x - placed.floor.position.x).abs() < 1.0e-3,
        "a rejected origin changed the floor"
    );

    let missing = client.post_raw(3, "tools/list", "{}", false);
    assert_eq!(missing.status, 400, "{}", missing.raw);

    let tools = client.rpc(4, "tools/list", "{}");
    for name in [
        "read_scene",
        "set_object",
        "spawn_object",
        "remove_object",
        "set_camera",
    ] {
        assert!(
            tools.raw.contains(&format!("\"name\":\"{name}\"")),
            "{}",
            tools.raw
        );
    }
    let resources = client.rpc(5, "resources/list", "{}");
    assert!(resources.raw.contains("genos://scene"), "{}", resources.raw);

    let read = client.scene_from_resource(6);
    assert_scene(&read, &host.drawn_scene(), &host.camera());
    assert!(read.walls.len() >= 2, "walls {}", read.walls.len());
    assert_eq!(read.solids.len(), 3);
    assert!(
        read.lights
            .iter()
            .any(|light| near_vec(light.position, Vec3::new(0.0, 7.0, 0.0))),
        "light is not near (0, 7, 0)"
    );
    let tool_read = client.tool_scene(7, "read_scene", "{}");
    assert_eq!(tool_read.walls.len(), read.walls.len());
    assert_eq!(tool_read.solids[0].handle, read.solids[0].handle);

    let wall = read.walls[0].handle.clone();
    let other = read.walls[1].clone();
    client.tool_scene(
        8,
        "set_object",
        &format!(r#"{{"handle":"{wall}","position":[3,0,5]}}"#),
    );
    let moved = client.scene_from_resource(9);
    let moved_wall = moved.object(&wall);
    assert!(
        near_vec(moved_wall.position, Vec3::new(3.0, 0.0, 5.0)),
        "{:?}",
        moved_wall.position
    );
    let still = moved.object(&other.handle);
    assert!(
        near_vec(still.position, other.position),
        "another wall moved"
    );
    assert_scene(&moved, &host.drawn_scene(), &host.camera());

    let spawned_wall = client.spawn(
        10,
        r#"{"kind":"wall","position":[-6,0,-3],"half_x":2,"half_z":0.15,"height":2.6}"#,
    );
    let spawned_solid = client.spawn(
        11,
        r#"{"kind":"solid","position":[4,0,4],"shape":"circle","size":0.8,"color":[0.2,0.3,0.4]}"#,
    );
    let spawned_light = client.spawn(
        12,
        r#"{"kind":"light","position":[1,3,1],"color":[0.5,0.6,0.7]}"#,
    );
    assert!(spawned_wall.starts_with("wall:"), "{spawned_wall}");
    assert!(spawned_solid.starts_with("solid:"), "{spawned_solid}");
    assert!(spawned_light.starts_with("light:"), "{spawned_light}");
    assert_ne!(spawned_wall, wall);

    let colored = client.call(
        13,
        "set_object",
        &format!(r#"{{"handle":"{spawned_wall}","absorption":1.5,"color":[0.2,0.4,0.6]}}"#),
    );
    assert!(colored.raw.contains("1.5"), "{}", colored.raw);
    client.tool_ok(
        14,
        "remove_object",
        &format!(r#"{{"handle":"{spawned_solid}"}}"#),
    );
    client.tool_ok(
        15,
        "set_camera",
        &format!(r#"{{"position":[-6,{CAMERA_HEIGHT},0],"yaw":0,"pitch":0}}"#),
    );

    let edited = client.scene_from_resource(16);
    assert!(
        edited.object_opt(&spawned_solid).is_none(),
        "removed solid is still in the read"
    );
    let barrier = edited.object(&spawned_wall);
    assert!(
        near(barrier.absorption, 1.5),
        "absorption {}",
        barrier.absorption
    );
    assert!(near_color(barrier.color, [0.2, 0.4, 0.6]));
    assert!(near(barrier.half_x, 2.0) && near(barrier.half_z, 0.15) && near(barrier.height, 2.6));
    assert!(edited.object_opt(&spawned_light).is_some());
    assert!(near_vec(
        edited.camera_position,
        Vec3::new(-6.0, CAMERA_HEIGHT, 0.0)
    ));
    assert!(near(edited.yaw, 0.0) && near(edited.pitch, 0.0));
    assert_scene(&edited, &host.drawn_scene(), &host.camera());

    let drawn = host.drawn_scene();
    let world = World::from_scene(drawn.clone());
    assert_eq!(world.scene.walls.len(), drawn.walls.len());
    assert_eq!(
        world.objects.len(),
        1 + drawn.walls.len() + drawn.solids.len()
    );
    assert!(
        world.objects.iter().any(|object| {
            near(object.bounds.center[0], -6.0)
                && near(object.bounds.center[2], -3.0)
                && near(object.bounds.half[0], 2.0)
                && near(object.bounds.half[2], 0.15)
                && near(object.bounds.center[1], 1.3)
        }),
        "the drawn world has no barrier at (-6, -3)"
    );
    assert!(
        world.objects.iter().any(|object| {
            near(object.bounds.center[0], 3.0) && near(object.bounds.center[2], 5.0)
        }),
        "the drawn world kept the old wall position"
    );

    let stepped = host.camera();
    let bodies = &stepped.physics.bodies[..];
    assert_eq!(bodies.len(), 1 + drawn.walls.len() + drawn.solids.len() + 1);
    assert!(
        bodies.iter().any(|body| {
            let Collider::Box { half_extents } = body.shape else {
                return false;
            };
            near(body.position.x, -6.0)
                && near(body.position.z, -3.0)
                && near(body.position.y, 1.3)
                && near(half_extents.x, 2.0)
                && near(half_extents.z, 0.15)
        }),
        "the capsule room has no barrier collider"
    );
    assert!(
        bodies
            .iter()
            .all(|body| !(near(body.position.x, 4.0) && near(body.position.z, 4.0))),
        "the removed solid still has a collider"
    );

    let mut walk_camera = host.camera();
    let mut walk_scene = host.drawn_scene();
    let walk = Actions {
        forward: 1.0,
        ..Actions::default()
    };
    for _ in 0..80 {
        update(&mut walk_camera, &mut walk_scene, &walk, 1.0 / 60.0);
    }
    assert!(
        walk_camera.position.z > -2.9 && walk_camera.position.z < -1.0,
        "eye z {} did not stop on the spawned wall",
        walk_camera.position.z
    );

    let mut events = client.open_events();
    assert!(
        events.try_event().is_none(),
        "the stream replayed an old update"
    );
    client.rpc(17, "resources/subscribe", r#"{"uri":"genos://scene"}"#);
    let light = edited.lights[0].handle.clone();
    client.tool_ok(
        18,
        "set_object",
        &format!(r#"{{"handle":"{light}","position":[0.4,7,0]}}"#),
    );
    let first = events.event();
    assert!(first.contains("notifications/resources/updated"), "{first}");
    assert!(first.contains("genos://scene"), "{first}");
    let after_tool = client.scene_from_resource(19);
    assert!(near(after_tool.object(&light).position.x, 0.4));

    host.with_frame(|scene, camera| {
        camera.yaw += 0.4;
        scene.lights[0].position.x = 1.5;
    });
    let second = events.event();
    assert!(
        second.contains("notifications/resources/updated"),
        "{second}"
    );
    let after_frame = client.scene_from_resource(20);
    assert!(near(after_frame.yaw, 0.4), "yaw {}", after_frame.yaw);
    assert!(near(after_frame.object(&light).position.x, 1.5));
    assert_scene(&after_frame, &host.drawn_scene(), &host.camera());
    drop(events);
    drop(client);
    drop(server);

    let mut gone = TcpStream::connect(addr_of(&url));
    if let Ok(stream) = gone.as_mut() {
        stream
            .set_read_timeout(Some(Duration::from_millis(300)))
            .ok();
        stream
            .set_write_timeout(Some(Duration::from_millis(300)))
            .ok();
        let _ = stream.write_all(b"POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let mut buf = [0u8; 64];
        let read = stream.read(&mut buf);
        assert!(
            read.unwrap_or(0) == 0,
            "the listener stayed up after the process dropped the server"
        );
    }
}

#[test]
fn the_lighting_report_is_served_on_the_bound_port() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("free port");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);
    let host = Host::new(shipped_scene(), Camera::opening());
    let server = Server::start_on(host, port).expect("server");
    assert!(server.url().contains(&format!(":{port}/mcp")), "{}", server.url());
    publish_lighting("stable: false\npending: 4\n");
    let (status, _, body) = round_trip(
        "127.0.0.1",
        port,
        b"GET /lighting HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    );
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("stable: false"), "{body}");
    assert!(body.contains("pending: 4"), "{body}");
}

#[test]
fn a_client_sets_the_lighting_and_reads_the_shot() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("free port");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);
    let host = Host::new(shipped_scene(), Camera::opening());
    let _server = Server::start_on(host, port).expect("server");
    let frame = std::thread::spawn(move || {
        for _ in 0..100 {
            if genos_mcp::take_shot_request() {
                genos_mcp::finish_shot(&[137, 80, 78, 71, 1, 2, 3, 4]);
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    let (status, _, body) = round_trip(
        "127.0.0.1",
        port,
        b"POST /settings HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: text/plain\r\nContent-Length: 11\r\nConnection: close\r\n\r\nsun=freeze\n",
    );
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("sun: freeze"), "{body}");
    let (status, headers, _) = round_trip(
        "127.0.0.1",
        port,
        b"GET /shot.png HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    );
    assert_eq!(status, 200, "shot status {status}");
    let kind = headers
        .iter()
        .find(|(name, _)| name == "content-type")
        .map(|(_, value)| value.as_str())
        .unwrap_or("");
    assert!(kind.starts_with("image/png"), "{kind}");
    let saved = std::fs::read(genos_mcp::shot_path()).expect("shot file");
    assert_eq!(&saved[..4], &[137, 80, 78, 71]);
    frame.join().expect("frame");
}

fn refuse_a_dead_process() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("fake listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let port = listener.local_addr().expect("addr").port();
    let path = discovery_dir().join("0.json");
    std::fs::create_dir_all(discovery_dir()).expect("discovery dir");
    std::fs::write(
        &path,
        format!(r#"{{"pid":0,"starttime":1,"url":"http://127.0.0.1:{port}/mcp"}}"#),
    )
    .expect("stale record");
    let _cleanup = RemoveFile(path.clone());
    let error = endpoint_at(&path).expect_err("dead process was accepted");
    assert!(error.contains("process is gone"), "{error}");
    assert!(
        listener.accept().is_err(),
        "the dead record opened a socket"
    );
}

struct RemoveFile(PathBuf);

impl Drop for RemoveFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct Client {
    host: String,
    port: u16,
    session: String,
}

struct Reply {
    status: u16,
    raw: String,
    json: Value,
}

struct Events {
    stream: TcpStream,
    buf: Vec<u8>,
}

impl Client {
    fn new(url: &str) -> Self {
        let (host, port) = addr_of(url);
        Self {
            host,
            port,
            session: String::new(),
        }
    }

    fn initialize(&mut self) -> String {
        let http = self.exchange(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"genos-test","version":"0.1.0"}}}"#,
            false,
            None,
        );
        assert_eq!(http.status, 200, "{}", http.body);
        self.session = http
            .headers
            .iter()
            .find(|(name, _)| name == "mcp-session-id")
            .map(|(_, value)| value.clone())
            .expect("Mcp-Session-Id");
        assert!(!self.session.is_empty());
        let json = parse(&http.body).expect("initialize json");
        assert_eq!(
            json.get("id").and_then(Value::as_i64),
            Some(1),
            "{}",
            http.body
        );
        http.body
    }

    fn notify_initialized(&self) {
        let reply = self.post_message(
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            true,
        );
        assert_eq!(reply.status, 202, "{}", reply.body);
    }

    fn rpc(&self, id: i64, method: &str, params: &str) -> Reply {
        let reply = self.post_raw(id, method, params, true);
        assert_eq!(reply.status, 200, "{} {method} {}", reply.status, reply.raw);
        assert!(reply.json.get("error").is_none(), "{}", reply.raw);
        reply
    }

    fn call(&self, id: i64, name: &str, arguments: &str) -> Reply {
        let reply = self.rpc(
            id,
            "tools/call",
            &format!(r#"{{"name":"{name}","arguments":{arguments}}}"#),
        );
        let is_error = reply
            .json
            .get("result")
            .and_then(|value| value.get("isError"))
            .and_then(Value::as_bool);
        assert_eq!(is_error, Some(false), "{}", reply.raw);
        reply
    }

    fn tool_ok(&self, id: i64, name: &str, arguments: &str) {
        let _ = self.call(id, name, arguments);
    }

    fn tool_scene(&self, id: i64, name: &str, arguments: &str) -> LiveScene {
        let reply = self.call(id, name, arguments);
        parse_scene(&tool_text(&reply.json))
    }

    fn spawn(&self, id: i64, arguments: &str) -> String {
        let reply = self.call(id, "spawn_object", arguments);
        let text = tool_text(&reply.json);
        let value = parse(&text).expect("spawn json");
        value
            .get("handle")
            .and_then(Value::as_str)
            .expect("spawn handle")
            .to_string()
    }

    fn scene_from_resource(&self, id: i64) -> LiveScene {
        let reply = self.rpc(id, "resources/read", r#"{"uri":"genos://scene"}"#);
        let text = reply
            .json
            .get("result")
            .and_then(|value| value.get("contents"))
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("text"))
            .and_then(Value::as_str)
            .expect("resource text")
            .to_string();
        assert!(reply.raw.contains("genos://scene"), "{}", reply.raw);
        parse_scene(&text)
    }

    fn post_origin(&self, id: i64, method: &str, params: &str, origin: &str) -> Http {
        let body =
            format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":{params}}}"#);
        self.exchange(&body, true, Some(origin))
    }

    fn post_raw(&self, id: i64, method: &str, params: &str, session: bool) -> Reply {
        let body =
            format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":{params}}}"#);
        let http = self.exchange(&body, session, None);
        let json = if http.body.is_empty() {
            Value::Null
        } else {
            parse(&http.body).unwrap_or(Value::Null)
        };
        Reply {
            status: http.status,
            raw: http.body,
            json,
        }
    }

    fn post_message(&self, body: &str, session: bool) -> Http {
        self.exchange(body, session, None)
    }

    fn exchange(&self, body: &str, session: bool, origin: Option<&str>) -> Http {
        let mut head = format!(
            "POST /mcp HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n",
            self.host,
            self.port,
            body.len()
        );
        if session {
            head.push_str("Mcp-Session-Id: ");
            head.push_str(&self.session);
            head.push_str("\r\nMcp-Protocol-Version: 2025-06-18\r\n");
        }
        if let Some(origin) = origin {
            head.push_str("Origin: ");
            head.push_str(origin);
            head.push_str("\r\n");
        }
        head.push_str("\r\n");
        head.push_str(body);
        let (status, headers, raw) = round_trip(&self.host, self.port, head.as_bytes());
        Http {
            status,
            headers,
            body: raw,
        }
    }

    fn open_events(&self) -> Events {
        let mut stream = TcpStream::connect((self.host.as_str(), self.port)).expect("sse connect");
        stream.set_nodelay(true).expect("nodelay");
        stream
            .set_read_timeout(Some(Duration::from_millis(300)))
            .expect("timeout");
        let request = format!(
            "GET /mcp HTTP/1.1\r\nHost: {}:{}\r\nAccept: text/event-stream\r\nMcp-Session-Id: {}\r\nMcp-Protocol-Version: 2025-06-18\r\nConnection: keep-alive\r\n\r\n",
            self.host, self.port, self.session
        );
        stream.write_all(request.as_bytes()).expect("sse write");
        let mut events = Events {
            stream,
            buf: Vec::new(),
        };
        let (status, headers) = events.read_head();
        assert_eq!(status, 200, "sse status {status}");
        let content_type = headers
            .iter()
            .find(|(name, _)| name == "content-type")
            .map(|(_, value)| value.as_str())
            .unwrap_or("");
        assert!(
            content_type.starts_with("text/event-stream"),
            "{content_type}"
        );
        events
    }
}

struct Http {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Events {
    fn try_event(&mut self) -> Option<String> {
        self.read_chunk().ok()
    }

    fn event(&mut self) -> String {
        self.stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .expect("timeout");
        self.read_chunk().expect("resource update")
    }

    fn read_head(&mut self) -> (u16, Vec<(String, String)>) {
        loop {
            if let Some(end) = find_header_end(&self.buf) {
                let (status, headers, _) = split_http(&self.buf, end);
                self.buf.drain(..end);
                return (status, headers);
            }
            self.pull().expect("sse headers");
        }
    }

    fn read_chunk(&mut self) -> Result<String, ()> {
        loop {
            if let Some(line_end) = find_line(&self.buf) {
                let line =
                    String::from_utf8_lossy(&self.buf[..line_end.saturating_sub(2)]).into_owned();
                let size = usize::from_str_radix(line.trim(), 16).map_err(|_| ())?;
                let data_at = line_end;
                let need = data_at + size + 2;
                if self.buf.len() < need {
                    self.pull()?;
                    continue;
                }
                let data = String::from_utf8_lossy(&self.buf[data_at..data_at + size]).into_owned();
                self.buf.drain(..need);
                return Ok(data);
            }
            self.pull()?;
        }
    }

    fn pull(&mut self) -> Result<(), ()> {
        let mut tmp = [0u8; 2048];
        match self.stream.read(&mut tmp) {
            Ok(0) => Err(()),
            Ok(count) => {
                self.buf.extend_from_slice(&tmp[..count]);
                Ok(())
            }
            Err(err)
                if err.kind() == std::io::ErrorKind::WouldBlock
                    || err.kind() == std::io::ErrorKind::TimedOut =>
            {
                Err(())
            }
            Err(_) => Err(()),
        }
    }
}

fn round_trip(host: &str, port: u16, request: &[u8]) -> (u16, Vec<(String, String)>, String) {
    let mut stream = TcpStream::connect((host, port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("timeout");
    stream.set_nodelay(true).expect("nodelay");
    stream.write_all(request).expect("write");
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(count) => buf.extend_from_slice(&tmp[..count]),
            Err(err)
                if err.kind() == std::io::ErrorKind::WouldBlock
                    || err.kind() == std::io::ErrorKind::TimedOut =>
            {
                break;
            }
            Err(err) => panic!("read: {err}"),
        }
    }
    let end = find_header_end(&buf).expect("http headers");
    split_http(&buf, end)
}

fn split_http(buf: &[u8], end: usize) -> (u16, Vec<(String, String)>, String) {
    let head = String::from_utf8_lossy(&buf[..end.saturating_sub(4)]);
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let mut headers = Vec::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    let body = String::from_utf8_lossy(&buf[end..]).into_owned();
    (status, headers, body)
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

fn find_line(buf: &[u8]) -> Option<usize> {
    buf.windows(2)
        .position(|window| window == b"\r\n")
        .map(|index| index + 2)
}

fn addr_of(url: &str) -> (String, u16) {
    let rest = url.trim_start_matches("http://");
    let (host_port, _) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = host_port.split_once(':').expect("host:port");
    (host.to_string(), port.parse().expect("port"))
}

fn tool_text(json: &Value) -> String {
    json.get("result")
        .and_then(|value| value.get("content"))
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        .expect("tool text")
        .to_string()
}

#[derive(Clone)]
struct Placed {
    handle: String,
    position: Vec3,
    half_x: f32,
    half_z: f32,
    height: f32,
    size: f32,
    shape: String,
    color: [f32; 3],
    absorption: f32,
}

struct LiveScene {
    floor_position: Vec3,
    floor_half_x: f32,
    floor_half_z: f32,
    floor_color: [f32; 3],
    walls: Vec<Placed>,
    solids: Vec<Placed>,
    lights: Vec<Placed>,
    camera_position: Vec3,
    yaw: f32,
    pitch: f32,
    look: Vec3,
}

impl LiveScene {
    fn object(&self, handle: &str) -> &Placed {
        self.object_opt(handle)
            .unwrap_or_else(|| panic!("missing {handle}"))
    }

    fn object_opt(&self, handle: &str) -> Option<&Placed> {
        self.walls
            .iter()
            .chain(self.solids.iter())
            .chain(self.lights.iter())
            .find(|item| item.handle == handle)
    }
}

fn parse_scene(text: &str) -> LiveScene {
    let value = parse(text).expect("scene json");
    let floor = value.get("floor").expect("floor");
    let camera = value.get("camera").expect("camera");
    LiveScene {
        floor_position: vec3(floor.get("position").expect("floor position")),
        floor_half_x: num(floor.get("half_x").expect("half_x")),
        floor_half_z: num(floor.get("half_z").expect("half_z")),
        floor_color: color(floor.get("color").expect("floor color")),
        walls: value
            .get("walls")
            .and_then(Value::as_array)
            .expect("walls")
            .iter()
            .map(placed)
            .collect(),
        solids: value
            .get("solids")
            .and_then(Value::as_array)
            .expect("solids")
            .iter()
            .map(placed)
            .collect(),
        lights: value
            .get("lights")
            .and_then(Value::as_array)
            .expect("lights")
            .iter()
            .map(placed)
            .collect(),
        camera_position: vec3(camera.get("position").expect("camera position")),
        yaw: num(camera.get("yaw").expect("yaw")),
        pitch: num(camera.get("pitch").expect("pitch")),
        look: vec3(camera.get("look").expect("look")),
    }
}

fn placed(value: &Value) -> Placed {
    Placed {
        handle: value
            .get("handle")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        position: value.get("position").map(vec3).unwrap_or(Vec3::ZERO),
        half_x: value.get("half_x").map(num).unwrap_or(0.0),
        half_z: value.get("half_z").map(num).unwrap_or(0.0),
        height: value.get("height").map(num).unwrap_or(0.0),
        size: value.get("size").map(num).unwrap_or(0.0),
        shape: value
            .get("shape")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        color: value.get("color").map(color).unwrap_or([0.0, 0.0, 0.0]),
        absorption: value.get("absorption").map(num).unwrap_or(0.0),
    }
}

fn assert_scene(live: &LiveScene, scene: &Scene, camera: &Camera) {
    assert!(near_vec(live.floor_position, scene.floor.position));
    assert!(near(live.floor_half_x, scene.floor.half_x));
    assert!(near(live.floor_half_z, scene.floor.half_z));
    assert!(near_color(live.floor_color, scene.floor.color));
    assert_eq!(live.walls.len(), scene.walls.len());
    for (got, wall) in live.walls.iter().zip(&scene.walls) {
        assert!(got.handle.starts_with("wall:"), "{}", got.handle);
        assert!(near_vec(got.position, wall.position));
        assert!(near(got.half_x, wall.half_x) && near(got.half_z, wall.half_z));
        assert!(near(got.height, wall.height));
        assert!(near_color(got.color, wall.color));
        assert!(near(got.absorption, wall.absorption));
    }
    assert_eq!(live.solids.len(), scene.solids.len());
    for (got, solid) in live.solids.iter().zip(&scene.solids) {
        assert!(got.handle.starts_with("solid:"), "{}", got.handle);
        let shape = match solid.shape {
            genos_scene::Shape::Square => "square",
            genos_scene::Shape::Circle => "circle",
        };
        assert_eq!(got.shape, shape);
        assert!(near_vec(got.position, solid.position));
        assert!(near(got.size, solid.size) && near(got.height, solid.height));
        assert!(near_color(got.color, solid.color));
        assert!(near(got.absorption, solid.absorption));
    }
    assert_eq!(live.lights.len(), scene.lights.len());
    for (got, light) in live.lights.iter().zip(&scene.lights) {
        assert!(got.handle.starts_with("light:"), "{}", got.handle);
        assert!(near_vec(got.position, light.position));
        assert!(near_color(got.color, light.color));
    }
    assert!(near_vec(live.camera_position, camera.position));
    assert!(near(live.yaw, camera.yaw) && near(live.pitch, camera.pitch));
    assert!(near_vec(
        live.look,
        look_direction(camera.yaw, camera.pitch)
    ));
}

fn vec3(value: &Value) -> Vec3 {
    let items = value.as_array().expect("vec3");
    Vec3::new(num(&items[0]), num(&items[1]), num(&items[2]))
}

fn color(value: &Value) -> [f32; 3] {
    let items = value.as_array().expect("color");
    [num(&items[0]), num(&items[1]), num(&items[2])]
}

fn num(value: &Value) -> f32 {
    value.as_f64().expect("number") as f32
}

fn near(got: f32, expected: f32) -> bool {
    (got - expected).abs() < 1.0e-3
}

fn near_vec(got: Vec3, expected: Vec3) -> bool {
    near(got.x, expected.x) && near(got.y, expected.y) && near(got.z, expected.z)
}

fn near_color(got: [f32; 3], expected: [f32; 3]) -> bool {
    near(got[0], expected[0]) && near(got[1], expected[1]) && near(got[2], expected[2])
}
