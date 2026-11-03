use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Instant,
};
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
pub struct Options {
    pub task: String,
    pub binary: PathBuf,
    pub output: Option<PathBuf>,
}
impl Options {
    pub fn parse(args: Vec<String>) -> Result<Option<Self>> {
        if args.is_empty() || args.iter().any(|s| s == "--help" || s == "-h") {
            println!(
                "cargo xtask portable-smoke [--binary PATH]\ncargo xtask large-library-smoke [--binary PATH] [--output PATH]\nDefault: repository target/release/nocturne[.exe]. Build release explicitly first; never uses debug or builds the application. All fixtures are temporary, outside the repository, with empty PATH and a fresh CLI process per command."
            );
            return Ok(None);
        }
        let task = args[0].clone();
        if !["portable-smoke", "large-library-smoke"].contains(&task.as_str()) {
            return Err("unknown task".into());
        }
        let mut binary = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("target/release")
            .join(if cfg!(windows) {
                "nocturne.exe"
            } else {
                "nocturne"
            });
        let mut output = None;
        let mut i = 1;
        while i < args.len() {
            let flag = &args[i];
            if flag != "--binary"
                && !(flag == "--output" && task == "large-library-smoke")
            {
                return Err(format!("unknown option: {flag}").into());
            }
            i += 1;
            let value = args.get(i).ok_or("missing option value")?;
            if flag == "--binary" {
                binary = value.into()
            } else {
                output = Some(value.into())
            }
            i += 1;
        }
        Ok(Some(Self {
            task,
            binary,
            output,
        }))
    }
}
pub struct Harness {
    pub temp: tempfile::TempDir,
    pub binary: PathBuf,
    pub last_peak: std::cell::Cell<Option<usize>>,
}
impl Harness {
    pub fn new(binary: &Path) -> Result<Self> {
        if !binary.is_file() {
            return Err(format!("release binary missing: {}; build release explicitly or pass --binary PATH", binary.display()).into());
        }
        let temp = tempfile::Builder::new()
            .prefix("nocturne-xtask-")
            .tempdir()?;
        let copied = temp.path().join(if cfg!(windows) {
            "nocturne.exe"
        } else {
            "nocturne"
        });
        std::fs::copy(binary, &copied)?;
        Ok(Self {
            temp,
            binary: copied,
            last_peak: std::cell::Cell::new(None),
        })
    }
    pub fn path(&self, name: &str) -> PathBuf {
        self.temp.path().join(name)
    }
    pub fn command(&self) -> Command {
        let mut c = Command::new(&self.binary);
        c.current_dir(self.temp.path())
            .env("PATH", "")
            .env_remove("NOCTURNE_DB")
            .env_remove("NOCTURNE_NAMESPACE");
        c
    }
    pub fn raw(
        &self, db: &str, ns: &str, args: &[&str], body: Option<&str>,
        json: bool, expected: i32,
    ) -> Result<(std::process::Output, f64)> {
        let mut c = self.command();
        c.arg("--db").arg(self.path(db)).args(["--namespace", ns]);
        if json {
            c.arg("--json");
        }
        c.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let start = Instant::now();
        let mut child = c.spawn()?;
        #[cfg(windows)]
        let peak = windows_memory::Process::duplicate(&child)?;
        if let Some(body) = body {
            child.stdin.take().unwrap().write_all(body.as_bytes())?;
        } else {
            drop(child.stdin.take());
        }
        let output = child.wait_with_output()?;
        #[cfg(windows)]
        self.last_peak.set(Some(peak.peak()?));
        if output.status.code() != Some(expected) {
            return Err(format!(
                "{args:?}: status {}; stderr {}; stdout {}",
                output.status,
                String::from_utf8_lossy(&output.stderr),
                String::from_utf8_lossy(&output.stdout)
            )
            .into());
        }
        Ok((output, start.elapsed().as_secs_f64() * 1000.))
    }
    pub fn run(
        &self, db: &str, args: &[&str], body: Option<&str>,
    ) -> Result<(Value, f64)> {
        self.ns(db, "", args, body, 0)
    }
    pub fn ns(
        &self, db: &str, ns: &str, args: &[&str], body: Option<&str>,
        expected: i32,
    ) -> Result<(Value, f64)> {
        let (out, ms) = self.raw(db, ns, args, body, true, expected)?;
        Ok((serde_json::from_slice(&out.stdout)?, ms))
    }
    pub fn report(&self) -> Result<Value> {
        Ok(
            json!({"result":"passed", "binary_bytes":std::fs::metadata(&self.binary)?.len(), "binary_sha256":hash(&std::fs::read(&self.binary)?), "environment":"Temporary databases; executable copied outside repository; empty PATH; fresh process per CLI call", "platform":std::env::consts::OS}),
        )
    }
}
pub fn hash(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
pub fn median(samples: &mut [f64]) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}
pub fn text(path: &Path) -> &str {
    path.to_str().expect("UTF-8 temporary path")
}
#[cfg(windows)]
mod windows_memory {
    use std::{
        ffi::c_void,
        io,
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::Child,
    };
    #[repr(C)]
    struct Counters {
        cb: u32,
        faults: u32,
        values: [usize; 8],
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn DuplicateHandle(
            source: *mut c_void, handle: *mut c_void, target: *mut c_void,
            output: *mut *mut c_void, access: u32, inherit: i32, options: u32,
        ) -> i32;
    }
    #[link(name = "psapi")]
    unsafe extern "system" {
        fn GetProcessMemoryInfo(
            handle: *mut c_void, counters: *mut Counters, size: u32,
        ) -> i32;
    }
    pub struct Process(OwnedHandle);
    impl Process {
        pub fn duplicate(child: &Child) -> io::Result<Self> {
            let mut handle = std::ptr::null_mut();
            // Keep a process handle alive after wait_with_output closes Child's handle.
            unsafe {
                let current = GetCurrentProcess();
                if DuplicateHandle(
                    current,
                    child.as_raw_handle(),
                    current,
                    &mut handle,
                    0,
                    0,
                    2,
                ) == 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(Self(OwnedHandle::from_raw_handle(handle)))
            }
        }
        pub fn peak(&self) -> io::Result<usize> {
            let size = std::mem::size_of::<Counters>() as u32;
            let mut counters = Counters {
                cb: size,
                faults: 0,
                values: [0; 8],
            };
            unsafe {
                if GetProcessMemoryInfo(
                    self.0.as_raw_handle(),
                    &mut counters,
                    size,
                ) == 0
                {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(counters.values[0])
        }
    }
}
