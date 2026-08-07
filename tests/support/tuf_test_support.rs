#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use a3s_use_core::{
    CatalogArchive, CatalogAvailability, CatalogPackage, CatalogSurface, PluginCatalogRecord,
    PluginPermissionCeiling, PluginReleaseChannel, PluginSurfaceKind, PLUGIN_CATALOG_SCHEMA_V3,
    PLUGIN_PERMISSION_SCHEMA,
};
use olpc_cjson::CanonicalFormatter;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde::Serialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

pub(crate) const FUTURE: &str = "2999-01-01T00:00:00Z";
pub(crate) const EXPIRED: &str = "2000-01-01T00:00:00Z";
pub(crate) const PACKAGE_VERSION: &str = "0.1.1";

pub(crate) struct TestRepository {
    pub(crate) routes: HashMap<String, Vec<u8>>,
    pub(crate) root_sha256: String,
    pub(crate) target_name: String,
    pub(crate) target_sha256: String,
}

pub(crate) struct TestTarget {
    pub(crate) archive: Vec<u8>,
    pub(crate) target_name: String,
    pub(crate) custom: Option<Value>,
}

impl TestRepository {
    pub(crate) fn new(archive: Vec<u8>, metadata_version: u64, expires: &str) -> Self {
        Self::with_package_version(archive, PACKAGE_VERSION, metadata_version, expires)
    }

    pub(crate) fn with_package_version(
        archive: Vec<u8>,
        package_version: &str,
        metadata_version: u64,
        expires: &str,
    ) -> Self {
        Self::with_targets(
            vec![extension_target(archive, package_version)],
            metadata_version,
            expires,
        )
    }

    pub(crate) fn with_target_metadata(
        archive: Vec<u8>,
        target_name: String,
        custom: Value,
        metadata_version: u64,
        expires: &str,
    ) -> Self {
        Self::with_targets(
            vec![TestTarget {
                archive,
                target_name,
                custom: Some(custom),
            }],
            metadata_version,
            expires,
        )
    }

    pub(crate) fn with_targets(
        targets: Vec<TestTarget>,
        metadata_version: u64,
        expires: &str,
    ) -> Self {
        assert!(!targets.is_empty(), "a TUF test repository needs a target");
        let key = Ed25519KeyPair::from_seed_unchecked(&[7_u8; 32]).unwrap();
        let public = hex_lower(key.public_key().as_ref());
        let key_value = json!({
            "keytype": "ed25519",
            "scheme": "ed25519",
            "keyval": {"public": public}
        });
        let key_id = sha256(&canonical(&key_value));
        let role = json!({"keyids": [key_id.clone()], "threshold": 1});
        let mut keys = Map::new();
        keys.insert(key_id.clone(), key_value);
        let root_signed = json!({
            "_type": "root",
            "spec_version": "1.0.0",
            "consistent_snapshot": false,
            "version": 1,
            "expires": FUTURE,
            "keys": keys,
            "roles": {
                "root": role.clone(),
                "snapshot": role.clone(),
                "targets": role.clone(),
                "timestamp": role
            }
        });
        let root = signed_document(&key, &key_id, root_signed);
        let root_sha256 = sha256(&root);

        let mut targets_map = Map::new();
        let mut target_routes = Vec::new();
        for target in targets {
            let target_sha256 = sha256(&target.archive);
            let custom = target
                .custom
                .map(|metadata| json!({"a3s": metadata}))
                .unwrap_or_else(
                    || json!({"a3sPlanning": {"schema": "a3s.use.plugin-planning-target.v1"}}),
                );
            targets_map.insert(
                target.target_name.clone(),
                json!({
                    "length": target.archive.len(),
                    "hashes": {"sha256": target_sha256},
                    "custom": custom
                }),
            );
            target_routes.push((
                format!("/targets/{}", target.target_name),
                target.archive,
                target.target_name,
                target_sha256,
            ));
        }
        let targets_signed = json!({
            "_type": "targets",
            "spec_version": "1.0.0",
            "version": metadata_version,
            "expires": expires,
            "targets": targets_map
        });
        let targets = signed_document(&key, &key_id, targets_signed);
        let snapshot_signed = json!({
            "_type": "snapshot",
            "spec_version": "1.0.0",
            "version": metadata_version,
            "expires": expires,
            "meta": {
                "targets.json": {
                    "version": metadata_version,
                    "length": targets.len(),
                    "hashes": {"sha256": sha256(&targets)}
                }
            }
        });
        let snapshot = signed_document(&key, &key_id, snapshot_signed);
        let timestamp_signed = json!({
            "_type": "timestamp",
            "spec_version": "1.0.0",
            "version": metadata_version,
            "expires": expires,
            "meta": {
                "snapshot.json": {
                    "version": metadata_version,
                    "length": snapshot.len(),
                    "hashes": {"sha256": sha256(&snapshot)}
                }
            }
        });
        let timestamp = signed_document(&key, &key_id, timestamp_signed);

        let mut routes = HashMap::from([
            ("/metadata/root.json".to_string(), root),
            ("/metadata/timestamp.json".to_string(), timestamp),
            ("/metadata/snapshot.json".to_string(), snapshot),
            ("/metadata/targets.json".to_string(), targets),
        ]);
        for (route, archive, _, _) in &target_routes {
            routes.insert(route.clone(), archive.clone());
        }
        let (_, _, target_name, target_sha256) = target_routes
            .into_iter()
            .next()
            .expect("a TUF test repository needs a target");
        Self {
            routes,
            root_sha256,
            target_name,
            target_sha256,
        }
    }
}

pub(crate) fn extension_target(archive: Vec<u8>, package_version: &str) -> TestTarget {
    let target = host_target();
    let archive_name = format!("a3s-use-a3s-science-{package_version}-{target}.tar.gz");
    let target_name =
        format!("extensions/a3s/science/{package_version}/stable/{target}/{archive_name}");
    let (package_sha256, file_count, expanded_bytes, manifest) =
        expanded_archive_fingerprint(&archive);
    let permission_ceiling = PluginPermissionCeiling {
        schema: PLUGIN_PERMISSION_SCHEMA.to_string(),
        surfaces: Vec::new(),
    };
    let custom = serde_json::to_value(PluginCatalogRecord {
        schema: PLUGIN_CATALOG_SCHEMA_V3.to_string(),
        package_id: "a3s/science".to_string(),
        display_name: "A3S Science".to_string(),
        description: "Static scientific guidance for A3S agents.".to_string(),
        publisher: "a3s".to_string(),
        keywords: vec!["science".to_string()],
        categories: vec!["research".to_string()],
        version: package_version.to_string(),
        channel: PluginReleaseChannel::Stable,
        requires_use: ">=0.3.0, <0.4.0".to_string(),
        dependencies: Vec::new(),
        target: target.to_string(),
        surfaces: vec![CatalogSurface {
            kind: PluginSurfaceKind::Skill,
            id: "science".to_string(),
            optional: false,
            workload: None,
            mcp_transport: None,
            mcp_tool_count: None,
            okf_bundle: None,
            requires: Vec::new(),
        }],
        permission_ceiling_digest: permission_ceiling.descriptor_digest().unwrap(),
        permission_ceiling,
        planning: None,
        archive: CatalogArchive {
            target_name: target_name.clone(),
            length: archive.len() as u64,
            sha256: format!("sha256:{}", sha256(&archive)),
        },
        package: CatalogPackage {
            expanded_bytes,
            file_count,
            sha256: Some(format!("sha256:{package_sha256}")),
            manifest_sha256: Some(format!("sha256:{}", sha256(&manifest))),
        },
        license: "Apache-2.0".to_string(),
        repository: "https://github.com/A3S-Lab/Science".to_string(),
        availability: CatalogAvailability::Available,
    })
    .unwrap();

    TestTarget {
        archive,
        target_name,
        custom: Some(custom),
    }
}

pub(crate) fn package_directory_archive(root: &Path) -> Vec<u8> {
    let mut files = Vec::new();
    collect_fixture_files(root, root, &mut files);
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let encoder = flate2::GzBuilder::new()
        .mtime(0)
        .operating_system(255)
        .write(Vec::new(), flate2::Compression::best());
    let mut archive = tar::Builder::new(encoder);
    for (relative, path) in files {
        let body = std::fs::read(&path).unwrap();
        let archive_path = format!("package/{relative}");
        let mode = if relative.contains("/bin/") {
            0o755
        } else {
            0o644
        };
        let mut header = tar::Header::new_gnu();
        header.set_path(archive_path).unwrap();
        header.set_size(body.len() as u64);
        header.set_mode(mode);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        archive.append(&header, body.as_slice()).unwrap();
    }
    archive.into_inner().unwrap().finish().unwrap()
}

fn collect_fixture_files(root: &Path, directory: &Path, output: &mut Vec<(String, PathBuf)>) {
    let mut entries = std::fs::read_dir(directory)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if entry.file_type().unwrap().is_dir() {
            collect_fixture_files(root, &path, output);
        } else {
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .iter()
                .map(|segment| segment.to_str().unwrap())
                .collect::<Vec<_>>()
                .join("/");
            output.push((relative, path));
        }
    }
}

fn signed_document(key: &Ed25519KeyPair, key_id: &str, signed: Value) -> Vec<u8> {
    let signature = key.sign(&canonical(&signed));
    serde_json::to_vec(&json!({
        "signatures": [{"keyid": key_id, "sig": hex_lower(signature.as_ref())}],
        "signed": signed
    }))
    .unwrap()
}

fn canonical(value: &Value) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut serializer =
        serde_json::Serializer::with_formatter(&mut bytes, CanonicalFormatter::new());
    value.serialize(&mut serializer).unwrap();
    bytes
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

pub(crate) fn extension_archive(version: &str) -> Vec<u8> {
    let manifest = format!(
        "extension \"a3s/science\" {{\n  schema_version = 3\n  version        = \"{version}\"\n  route          = \"science\"\n  requires_use   = \">=0.3.0, <0.4.0\"\n  actions        = [\"read\"]\n\n  repository {{\n    url      = \"https://github.com/A3S-Lab/Science\"\n    revision = \"0123456789abcdef0123456789abcdef01234567\"\n  }}\n\n  skill \"science\" {{\n    path          = \"skills/science/SKILL.md\"\n    requires_tool = []\n    requires_mcp  = []\n    requires_okf  = []\n    optional      = false\n  }}\n}}\n"
    );
    let mut bytes = Vec::new();
    {
        let encoder = flate2::write::GzEncoder::new(&mut bytes, flate2::Compression::default());
        let mut archive = tar::Builder::new(encoder);
        append_tar_file(
            &mut archive,
            "package/a3s-use-extension.acl",
            0o644,
            manifest.as_bytes(),
        );
        append_tar_file(
            &mut archive,
            "package/README.md",
            0o644,
            b"# A3S Science\n\nStatic scientific guidance fixture.\n",
        );
        append_tar_file(
            &mut archive,
            "package/skills/science/SKILL.md",
            0o644,
            b"---\nname: science\ndescription: Science fixture\n---\n# Science\n",
        );
        archive.finish().unwrap();
    }
    bytes
}

fn expanded_archive_fingerprint(archive: &[u8]) -> (String, u64, u64, Vec<u8>) {
    let decoder = flate2::read::GzDecoder::new(Cursor::new(archive));
    let mut input = tar::Archive::new(decoder);
    let mut files = Vec::new();
    for entry in input.entries().unwrap() {
        let mut entry = entry.unwrap();
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path().unwrap().into_owned();
        let relative = path.strip_prefix("package").unwrap();
        let normalized = relative
            .iter()
            .map(|segment| segment.to_str().unwrap())
            .collect::<Vec<_>>()
            .join("/");
        let mut body = Vec::new();
        entry.read_to_end(&mut body).unwrap();
        files.push((normalized, body));
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let mut digest = Sha256::new();
    digest.update(b"a3s-use-expanded-package-v1\0");
    let mut expanded_bytes = 0_u64;
    let mut manifest = None;
    for (path, body) in &files {
        let size = body.len() as u64;
        expanded_bytes += size;
        digest.update((path.len() as u64).to_be_bytes());
        digest.update(path.as_bytes());
        digest.update(size.to_be_bytes());
        digest.update(body);
        if path == "a3s-use-extension.acl" {
            manifest = Some(body.clone());
        }
    }
    (
        format!("{:x}", digest.finalize()),
        files.len() as u64,
        expanded_bytes,
        manifest.expect("the test package archive must contain its manifest"),
    )
}

fn append_tar_file<W: Write>(archive: &mut tar::Builder<W>, path: &str, mode: u32, body: &[u8]) {
    let mut header = tar::Header::new_gnu();
    header.set_path(path).unwrap();
    header.set_size(body.len() as u64);
    header.set_mode(mode);
    header.set_cksum();
    archive.append(&header, body).unwrap();
}

pub(crate) fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

pub(crate) fn host_target() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("macos", "x86_64") => "darwin-x86_64",
        ("linux", "aarch64") => "linux-arm64",
        ("linux", "x86_64") => "linux-x86_64",
        ("windows", "x86_64") => "windows-x86_64",
        (os, arch) => panic!("unsupported TUF test target {os}-{arch}"),
    }
}

pub(crate) struct TestServer {
    base_url: String,
    routes: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl TestServer {
    pub(crate) fn start(routes: HashMap<String, Vec<u8>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base_url = format!("http://{}/", listener.local_addr().unwrap());
        let routes = Arc::new(Mutex::new(routes));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_routes = Arc::clone(&routes);
        let thread_requests = Arc::clone(&requests);
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        let routes = Arc::clone(&thread_routes);
                        let requests = Arc::clone(&thread_requests);
                        std::thread::spawn(move || serve(stream, &routes, &requests));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            base_url,
            routes,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }

    pub(crate) fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }

    pub(crate) fn clear_requests(&self) {
        self.requests.lock().unwrap().clear();
    }

    pub(crate) fn replace_routes(&self, routes: HashMap<String, Vec<u8>>) {
        *self.routes.lock().unwrap() = routes;
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(
    mut stream: TcpStream,
    routes: &Mutex<HashMap<String, Vec<u8>>>,
    requests: &Mutex<Vec<String>>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut buffer = [0_u8; 8192];
    let Ok(size) = stream.read(&mut buffer) else {
        return;
    };
    let request = String::from_utf8_lossy(&buffer[..size]);
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_string();
    requests.lock().unwrap().push(path.clone());
    let body = routes.lock().unwrap().get(&path).cloned();
    let (status, body) = body
        .as_deref()
        .map(|body| ("200 OK", body))
        .unwrap_or(("404 Not Found", b"not found"));
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    if stream.write_all(header.as_bytes()).is_ok() && stream.write_all(body).is_ok() {
        let _ = stream.flush();
        let _ = stream.shutdown(Shutdown::Write);
    }
}
