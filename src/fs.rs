//! Node.js の `fs`(同期 API)相当の骨格。
//!
//! [`crate::resolve`] の `StdFileSystem` は「モジュール解決のための存在判定」
//! だけに特化した最小限のファイルシステム抽象だったが、本モジュールは
//! それとは独立に、Node の `fs.readFileSync`/`fs.writeFileSync`/
//! `fs.readdirSync`/`fs.statSync` 等に相当する**汎用の同期ファイル I/O**を
//! 提供する。既存の Node.js 実装コードは一切流用せず、`std::fs` の薄い
//! ラッパーとして一から実装する(このクレートは依存クレートゼロの方針を
//! 維持するため、非同期版は `tokio::fs` 依存を追加しない限りスコープ外
//! ——v0.1.0 は Node の `fs`(同期)相当のみとする)。
//!
//! 参考にした Node.js の概念(実装コードは一切流用していない):
//! - `fs.readFileSync(path)` / `fs.readFileSync(path, 'utf8')`
//! - `fs.writeFileSync(path, data)`
//! - `fs.existsSync(path)`
//! - `fs.mkdirSync(path, { recursive: true })`
//! - `fs.readdirSync(path)`(エントリ名の一覧、`.`/`..` は含めない)
//! - `fs.statSync(path)` → `is_file`/`is_dir`/`len` を持つ [`Metadata`]
//! - `fs.unlinkSync(path)` / `fs.rmdirSync(path)` / `fs.rmSync(path, { recursive: true })`
//!
//! v0.1.0 で**モデル化していない**もの: シンボリックリンク固有の挙動、
//! パーミッション操作(`chmod` 等)、ストリーム API、非同期版
//! (`fs.promises`/コールバック版)。

use std::io;
use std::path::Path;

/// ファイル I/O 操作の失敗を表す。`std::io::Error` をそのまま包んで
/// 呼び出し元パスの情報を添える(Node の `Error.code`/`path` に相当)。
#[derive(Debug)]
pub struct FsError {
    /// 操作対象のパス。
    pub path: String,
    /// 元となった I/O エラー。
    pub source: io::Error,
}

impl std::fmt::Display for FsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.source)
    }
}

impl std::error::Error for FsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

fn wrap(path: &str, source: io::Error) -> FsError {
    FsError {
        path: path.to_string(),
        source,
    }
}

/// `fs.statSync(path)` 相当の最小メタデータ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metadata {
    is_file: bool,
    is_dir: bool,
    len: u64,
}

impl Metadata {
    /// 通常ファイルなら true。
    pub fn is_file(&self) -> bool {
        self.is_file
    }

    /// ディレクトリなら true。
    pub fn is_dir(&self) -> bool {
        self.is_dir
    }

    /// バイト単位のサイズ(ディレクトリの場合は OS 依存の値、通常は無視してよい)。
    pub fn len(&self) -> u64 {
        self.len
    }

    /// サイズが 0 なら true(Clippy の `len_without_is_empty` 慣習に合わせる)。
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// `fs.readFileSync(path)` 相当。ファイル内容を生バイト列で読む。
pub fn read_file<P: AsRef<Path>>(path: P) -> Result<Vec<u8>, FsError> {
    let path = path.as_ref();
    std::fs::read(path).map_err(|e| wrap(&path.to_string_lossy(), e))
}

/// `fs.readFileSync(path, 'utf8')` 相当。UTF-8 文字列として読む。
/// 不正な UTF-8 の場合は [`FsError`] の `source` が
/// `io::ErrorKind::InvalidData` になる。
pub fn read_to_string<P: AsRef<Path>>(path: P) -> Result<String, FsError> {
    let path = path.as_ref();
    std::fs::read_to_string(path).map_err(|e| wrap(&path.to_string_lossy(), e))
}

/// `fs.writeFileSync(path, data)` 相当。既存ファイルは上書きする。
pub fn write_file<P: AsRef<Path>, C: AsRef<[u8]>>(path: P, contents: C) -> Result<(), FsError> {
    let path = path.as_ref();
    std::fs::write(path, contents).map_err(|e| wrap(&path.to_string_lossy(), e))
}

/// `fs.existsSync(path)` 相当。
pub fn exists<P: AsRef<Path>>(path: P) -> bool {
    path.as_ref().exists()
}

/// `fs.mkdirSync(path)` 相当(親ディレクトリが存在しない場合は失敗)。
pub fn mkdir<P: AsRef<Path>>(path: P) -> Result<(), FsError> {
    let path = path.as_ref();
    std::fs::create_dir(path).map_err(|e| wrap(&path.to_string_lossy(), e))
}

/// `fs.mkdirSync(path, { recursive: true })` 相当(祖先ディレクトリも作る)。
pub fn mkdir_recursive<P: AsRef<Path>>(path: P) -> Result<(), FsError> {
    let path = path.as_ref();
    std::fs::create_dir_all(path).map_err(|e| wrap(&path.to_string_lossy(), e))
}

/// `fs.readdirSync(path)` 相当。エントリ名(パスではなくファイル名のみ)を
/// ソート済みで返す(順序を決定的にするため。Node は OS 依存順だが、
/// テスト容易性のためソートしておく)。
pub fn read_dir<P: AsRef<Path>>(path: P) -> Result<Vec<String>, FsError> {
    let path = path.as_ref();
    let entries = std::fs::read_dir(path).map_err(|e| wrap(&path.to_string_lossy(), e))?;
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| wrap(&path.to_string_lossy(), e))?;
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    Ok(names)
}

/// `fs.statSync(path)` 相当。
pub fn stat<P: AsRef<Path>>(path: P) -> Result<Metadata, FsError> {
    let path = path.as_ref();
    let meta = std::fs::metadata(path).map_err(|e| wrap(&path.to_string_lossy(), e))?;
    Ok(Metadata {
        is_file: meta.is_file(),
        is_dir: meta.is_dir(),
        len: meta.len(),
    })
}

/// `fs.unlinkSync(path)` 相当。通常ファイルを削除する。
pub fn remove_file<P: AsRef<Path>>(path: P) -> Result<(), FsError> {
    let path = path.as_ref();
    std::fs::remove_file(path).map_err(|e| wrap(&path.to_string_lossy(), e))
}

/// `fs.rmdirSync(path)` 相当。空ディレクトリを削除する。
pub fn remove_dir<P: AsRef<Path>>(path: P) -> Result<(), FsError> {
    let path = path.as_ref();
    std::fs::remove_dir(path).map_err(|e| wrap(&path.to_string_lossy(), e))
}

/// `fs.rmSync(path, { recursive: true })` 相当。中身ごとディレクトリを削除する。
pub fn remove_dir_all<P: AsRef<Path>>(path: P) -> Result<(), FsError> {
    let path = path.as_ref();
    std::fs::remove_dir_all(path).map_err(|e| wrap(&path.to_string_lossy(), e))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト用の一時ディレクトリ。`Drop` で自動クリーンアップする。
    /// `resolve.rs` の同名ヘルパーと同じ設計(このモジュール単体で
    /// 完結させるため独立して定義する)。
    struct TempDir {
        path: std::path::PathBuf,
    }

    impl TempDir {
        fn new(name: &str) -> Self {
            let mut path = std::env::temp_dir();
            path.push(format!(
                "rnodejs-fs-test-{name}-{}-{}",
                std::process::id(),
                name.len()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }

        fn join(&self, rel: &str) -> std::path::PathBuf {
            self.path.join(rel)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn write_then_read_round_trips_utf8_content() {
        let tmp = TempDir::new("round-trip");
        let file = tmp.join("hello.txt");

        write_file(&file, "こんにちは, Node!").unwrap();
        let got = read_to_string(&file).unwrap();

        assert_eq!(got, "こんにちは, Node!");
    }

    #[test]
    fn write_then_read_round_trips_raw_bytes() {
        let tmp = TempDir::new("round-trip-bytes");
        let file = tmp.join("bin.dat");
        let payload = vec![0u8, 1, 2, 255, 254, 253];

        write_file(&file, &payload).unwrap();
        let got = read_file(&file).unwrap();

        assert_eq!(got, payload);
    }

    #[test]
    fn exists_reflects_real_filesystem_state() {
        let tmp = TempDir::new("exists");
        let file = tmp.join("maybe.txt");

        assert!(!exists(&file));
        write_file(&file, b"x").unwrap();
        assert!(exists(&file));
    }

    #[test]
    fn mkdir_recursive_creates_nested_ancestors() {
        let tmp = TempDir::new("mkdir-recursive");
        let nested = tmp.join("a/b/c");

        assert!(mkdir(&nested).is_err(), "非再帰版は祖先が無ければ失敗する");
        mkdir_recursive(&nested).unwrap();

        assert!(nested.is_dir());
    }

    #[test]
    fn read_dir_lists_entries_sorted() {
        let tmp = TempDir::new("readdir");
        write_file(tmp.join("b.txt"), b"b").unwrap();
        write_file(tmp.join("a.txt"), b"a").unwrap();
        mkdir(tmp.join("c_dir")).unwrap();

        let names = read_dir(&tmp.path).unwrap();

        assert_eq!(names, vec!["a.txt", "b.txt", "c_dir"]);
    }

    #[test]
    fn stat_reports_file_vs_dir_and_size() {
        let tmp = TempDir::new("stat");
        let file = tmp.join("data.bin");
        write_file(&file, b"12345").unwrap();

        let file_meta = stat(&file).unwrap();
        assert!(file_meta.is_file());
        assert!(!file_meta.is_dir());
        assert_eq!(file_meta.len(), 5);
        assert!(!file_meta.is_empty());

        let dir_meta = stat(&tmp.path).unwrap();
        assert!(dir_meta.is_dir());
        assert!(!dir_meta.is_file());
    }

    #[test]
    fn remove_file_deletes_real_file() {
        let tmp = TempDir::new("remove-file");
        let file = tmp.join("gone.txt");
        write_file(&file, b"bye").unwrap();
        assert!(exists(&file));

        remove_file(&file).unwrap();

        assert!(!exists(&file));
    }

    #[test]
    fn remove_dir_requires_empty_directory() {
        let tmp = TempDir::new("remove-dir-empty");
        let dir = tmp.join("empty_dir");
        mkdir(&dir).unwrap();

        remove_dir(&dir).unwrap();

        assert!(!dir.exists());
    }

    #[test]
    fn remove_dir_all_deletes_nested_contents() {
        let tmp = TempDir::new("remove-dir-all");
        let nested = tmp.join("x/y/z");
        mkdir_recursive(&nested).unwrap();
        write_file(nested.join("leaf.txt"), b"leaf").unwrap();

        remove_dir_all(tmp.join("x")).unwrap();

        assert!(!tmp.join("x").exists());
    }

    #[test]
    fn read_file_reports_not_found_error_with_path() {
        let tmp = TempDir::new("missing");
        let file = tmp.join("does-not-exist.txt");

        let err = read_file(&file).unwrap_err();

        assert_eq!(err.source.kind(), io::ErrorKind::NotFound);
        assert!(err.path.contains("does-not-exist.txt"));
    }
}
