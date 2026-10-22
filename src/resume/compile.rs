//! Runs pdflatex on an uploaded resume, sandboxed as far as TeX allows, and
//! reports how many pages came out.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::Semaphore;

const COMPILE_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Debug)]
pub enum CompileError {
    /// pdflatex isn't installed (or `PDFLATEX` points somewhere wrong).
    NotInstalled,
    /// bubblewrap isn't installed, so compiling untrusted LaTeX isn't safe.
    NoSandbox,
    /// The sandbox itself failed to start; holds its error output.
    Sandbox(String),
    /// LaTeX reported an error; holds the relevant lines from the log.
    Latex(String),
    Timeout,
    Io(std::io::Error),
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompileError::NotInstalled => write!(f, "pdflatex is not installed on the bot's host"),
            CompileError::NoSandbox => write!(
                f,
                "bubblewrap isn't installed on the bot's host, so resumes aren't compiled (`sudo apt install bubblewrap`, or set RESUME_SANDBOX=off if only you use this bot)"
            ),
            CompileError::Sandbox(err) => write!(f, "the LaTeX sandbox failed to start: {err}"),
            CompileError::Latex(log) => write!(f, "LaTeX error:\n{log}"),
            CompileError::Timeout => write!(f, "LaTeX took longer than {}s and was stopped", COMPILE_TIMEOUT.as_secs()),
            CompileError::Io(e) => write!(f, "could not run pdflatex: {e}"),
        }
    }
}

impl std::error::Error for CompileError {}

pub struct Compiled {
    pub pdf: Vec<u8>,
    pub pages: u32,
    #[cfg_attr(not(test), allow(dead_code))]
    log: String,
}

/// Finds an executable by name on PATH (or accepts a path as-is).
fn which(name: &str) -> Option<PathBuf> {
    let candidate = Path::new(name);
    if candidate.components().count() > 1 {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

/// The real pdftex binary behind `pdflatex` (which is usually a symlink).
fn engine() -> Option<PathBuf> {
    let name = std::env::var("PDFLATEX").unwrap_or_else(|_| "pdflatex".to_string());
    which(&name).and_then(|p| std::fs::canonicalize(p).ok())
}

/// `None` means compile without a sandbox (only when RESUME_SANDBOX=off).
fn sandbox() -> Result<Option<PathBuf>, CompileError> {
    if std::env::var("RESUME_SANDBOX").is_ok_and(|v| v.eq_ignore_ascii_case("off")) {
        return Ok(None);
    }
    let name = std::env::var("BWRAP").unwrap_or_else(|_| "bwrap".to_string());
    which(&name).map(Some).ok_or(CompileError::NoSandbox)
}

/// A TeX installation outside /usr (TinyTeX, a TUG install in $HOME) has to be
/// made visible inside the sandbox: `<root>/bin/<arch>/pdftex` -> `<root>`.
fn tex_root(binary: &Path) -> Option<PathBuf> {
    if binary.starts_with("/usr") {
        return None;
    }
    binary.ancestors().find(|a| a.file_name().is_some_and(|n| n == "bin")).and_then(|bin| bin.parent()).map(Path::to_path_buf)
}

/// One compile at a time keeps a Raspberry Pi responsive.
fn permits() -> &'static Semaphore {
    static PERMITS: OnceLock<Semaphore> = OnceLock::new();
    PERMITS.get_or_init(|| Semaphore::new(1))
}

/// Primitives and macros that read or write files or run programs. The
/// bubblewrap sandbox is the real protection; this list just gives users a
/// clear message instead of a confusing LaTeX error.
const FORBIDDEN: &[&str] = &[
    "\\write18", "\\immediate", "\\openin", "\\openout", "\\read", "\\directlua", "\\luaexec", "\\pdffiledump",
    "\\pdfmdfivesum", "\\pdffilesize", "\\pdffilemoddate", "\\pdfobj", "\\pdfximage", "\\verbatiminput", "\\VerbatimInput",
    "\\lstinputlisting", "\\inputminted", "\\includegraphics", "\\includepdf", "\\include{", "\\InputIfFileExists",
    "\\IfFileExists", "\\@input", "\\@@input", "\\@iinput", "\\CatchFile", "\\file_", "\\ior_", "\\iow_", "\\sys_shell",
    "\\catcode", "\\csname", "\\usepackage{shellesc}", "\\ShellEscape", "\\special",
];

/// Rejects LaTeX that tries to touch the host. Returns a user-facing reason.
pub fn check_safe(tex: &str) -> Result<(), String> {
    for bad in FORBIDDEN {
        if tex.contains(bad) {
            return Err(format!("`{bad}` isn't allowed in uploaded resumes."));
        }
    }
    let mut rest = tex;
    while let Some(i) = rest.find("\\input") {
        let after = &rest[i + "\\input".len()..];
        let arg: String = after
            .trim_start()
            .trim_start_matches('{')
            .chars()
            .take_while(|c| !matches!(c, '}' | '\n' | ' '))
            .collect();
        if arg.is_empty() || !arg.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err(format!("`\\input{{{arg}}}` isn't allowed; only plain package names like `glyphtounicode` are."));
        }
        rest = after;
    }
    Ok(())
}

fn scratch_dir() -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("jake-resume-{}-{n}", std::process::id()))
}

/// "Output written on resume.pdf (2 pages, 51234 bytes)."
fn page_count(log: &str) -> Option<u32> {
    let idx = log.find("Output written on")?;
    let rest = &log[idx..];
    let open = rest.find('(')?;
    let digits: String = rest[open + 1..].chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// The `! ...` error lines plus a little context, for showing to the user.
fn error_excerpt(log: &str) -> String {
    let lines: Vec<&str> = log.lines().collect();
    let mut out: Vec<String> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.starts_with('!') {
            out.extend(lines[i..(i + 3).min(lines.len())].iter().map(|l| l.to_string()));
            if out.len() >= 9 {
                break;
            }
        }
    }
    if out.is_empty() {
        out = lines.iter().rev().take(6).rev().map(|l| l.to_string()).collect();
    }
    out.join("\n").chars().take(900).collect()
}

pub async fn compile(tex: &str) -> Result<Compiled, CompileError> {
    check_safe(tex).map_err(CompileError::Latex)?;
    compile_unchecked(tex).await
}

async fn compile_unchecked(tex: &str) -> Result<Compiled, CompileError> {
    let engine = engine().ok_or(CompileError::NotInstalled)?;
    let sandbox = sandbox()?;
    let _permit = permits().acquire().await.expect("semaphore never closes");

    let dir = scratch_dir();
    tokio::fs::create_dir_all(&dir).await.map_err(CompileError::Io)?;
    let result = run_in(&dir, tex, &engine, sandbox.as_deref()).await;
    let _ = tokio::fs::remove_dir_all(&dir).await;
    result
}

/// Environment for pdflatex: TeX's own "paranoid" file mode and no font
/// generation helpers. Defence in depth; the sandbox is what hides the host.
const TEX_ENV: &[(&str, &str)] = &[
    ("openin_any", "p"),
    ("openout_any", "p"),
    ("shell_escape", "f"),
    ("MKTEXTFM", "0"),
    ("MKTEXPK", "0"),
    ("MKTEXMF", "0"),
    ("MKTEXTEX", "0"),
];

fn command(dir: &Path, engine: &Path, sandbox: Option<&Path>) -> Command {
    // Called as pdftex directly (symlinks are resolved), so name the format.
    let tex_args = ["-progname=pdflatex", "-fmt=pdflatex", "-interaction=nonstopmode", "-halt-on-error", "-no-shell-escape", "resume.tex"];
    let Some(bwrap) = sandbox else {
        let mut cmd = Command::new(engine);
        cmd.args(tex_args).current_dir(dir).envs(TEX_ENV.iter().copied());
        return cmd;
    };

    // Only the TeX installation (read-only) and the scratch directory exist
    // inside; the bot's home, .env and network are invisible.
    let mut cmd = Command::new(bwrap);
    cmd.args(["--unshare-all", "--die-with-parent", "--new-session", "--ro-bind", "/usr", "/usr"]);
    for path in ["/bin", "/lib", "/lib64", "/lib32", "/etc/ld.so.cache", "/etc/texmf", "/var/lib/texmf"] {
        cmd.args(["--ro-bind-try", path, path]);
    }
    if let Some(root) = tex_root(engine) {
        cmd.arg("--ro-bind").arg(&root).arg(&root);
    }
    cmd.args(["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"]);
    cmd.arg("--bind").arg(dir).arg("/work");
    cmd.args(["--chdir", "/work", "--clearenv", "--setenv", "HOME", "/work", "--setenv", "PATH", "/usr/bin:/bin"]);
    for (key, value) in TEX_ENV {
        cmd.args(["--setenv", key, value]);
    }
    cmd.arg(engine).args(tex_args);
    cmd
}

async fn run_in(dir: &Path, tex: &str, engine: &Path, sandbox: Option<&Path>) -> Result<Compiled, CompileError> {
    tokio::fs::write(dir.join("resume.tex"), tex).await.map_err(CompileError::Io)?;

    let child = command(dir, engine, sandbox)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let child = match child {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(if sandbox.is_some() { CompileError::NoSandbox } else { CompileError::NotInstalled });
        }
        Err(e) => return Err(CompileError::Io(e)),
    };

    let output = match tokio::time::timeout(COMPILE_TIMEOUT, child.wait_with_output()).await {
        Ok(output) => output.map_err(CompileError::Io)?,
        // Dropping the future drops the child, and kill_on_drop stops it.
        Err(_) => return Err(CompileError::Timeout),
    };

    let log = tokio::fs::read(dir.join("resume.log")).await.ok().map(|b| String::from_utf8_lossy(&b).into_owned());
    if !output.status.success() {
        return Err(match log {
            Some(log) => CompileError::Latex(error_excerpt(&log)),
            // No log means pdflatex never ran: the sandbox couldn't start.
            None => CompileError::Sandbox(String::from_utf8_lossy(&output.stderr).chars().take(300).collect()),
        });
    }
    let pdf = tokio::fs::read(dir.join("resume.pdf")).await.map_err(CompileError::Io)?;
    let log = log.unwrap_or_default();
    let pages = page_count(&log).unwrap_or(1);
    Ok(Compiled { pdf, pages, log })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safety_checks() {
        assert!(check_safe("\\input{glyphtounicode}\n\\begin{document}hi\\end{document}").is_ok());
        assert!(check_safe("\\input{/etc/passwd}").is_err());
        assert!(check_safe("\\input{../.env}").is_err());
        assert!(check_safe("\\immediate\\write18{rm -rf ~}").is_err());
        assert!(check_safe("\\verbatiminput{.env}").is_err());
        assert!(check_safe("\\InputIfFileExists{/etc/passwd}{}{}").is_err());
        assert!(check_safe("\\let\\x=\\input \\x{/etc/passwd}").is_err());
    }

    #[test]
    fn finds_tex_root() {
        assert_eq!(tex_root(Path::new("/home/pi/.TinyTeX/bin/aarch64-linux/pdftex")), Some(PathBuf::from("/home/pi/.TinyTeX")));
        assert_eq!(tex_root(Path::new("/usr/bin/pdftex")), None);
    }

    /// The sandbox must hide files outside the scratch directory, even from
    /// LaTeX that gets past `check_safe`. Skipped without pdflatex/bubblewrap.
    #[tokio::test]
    async fn sandbox_hides_host_files() {
        let secret = std::env::temp_dir().join("jake_sandbox_probe.txt");
        std::fs::write(&secret, "TOP-SECRET-TOKEN").unwrap();
        let home_probe = std::env::var("HOME").map(|h| format!("{h}/.bashrc")).unwrap_or_default();
        let tex = format!(
            "\\documentclass{{article}}\\begin{{document}}\
             \\IfFileExists{{{}}}{{\\typeout{{PROBE-ABS-FOUND}}}}{{\\typeout{{PROBE-ABS-MISSING}}}}\
             \\IfFileExists{{{home_probe}}}{{\\typeout{{PROBE-HOME-FOUND}}}}{{\\typeout{{PROBE-HOME-MISSING}}}}\
             x\\end{{document}}",
            secret.display()
        );
        let result = compile_unchecked(&tex).await;
        let _ = std::fs::remove_file(&secret);
        match result {
            Err(CompileError::NotInstalled | CompileError::NoSandbox) => eprintln!("pdflatex or bubblewrap missing; skipping"),
            Err(e) => panic!("probe should compile: {e}"),
            Ok(c) => {
                assert!(c.log.contains("PROBE-ABS-MISSING"), "absolute path visible inside sandbox");
                assert!(!c.log.contains("PROBE-HOME-FOUND"), "home directory visible inside sandbox");
            }
        }
    }

    #[test]
    fn reads_page_count() {
        assert_eq!(page_count("blah\nOutput written on resume.pdf (2 pages, 51234 bytes).\n"), Some(2));
        assert_eq!(page_count("Output written on resume.pdf (1 page, 1 bytes)."), Some(1));
        assert_eq!(page_count("No pages of output."), None);
    }
}
