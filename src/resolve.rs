//! CommonJS 的なモジュール解決アルゴリズムの骨格。
//!
//! Node.js の `require(path)` が内部で行うパス解決規則
//! (相対パス・拡張子補完 `.js`/`.json`・ディレクトリの `index`・
//! `node_modules` 探索)を、**実際のファイルシステムアクセスから切り離した
//! 純粋関数**として実装する。ファイルの存在判定は [`FileSystem`] トレイト
//! 経由で抽象化してあるので、テストではインメモリの [`MapFileSystem`] を
//! 差し込むだけで、実 I/O を一切伴わずに解決規則そのものを検証できる。
//!
//! 参考にした Node.js の概念(実装コードは一切流用していない):
//! `require` の解決は概ね `LOAD_AS_FILE` → `LOAD_AS_DIRECTORY` →
//! `LOAD_NODE_MODULES` の順に試みる。本モジュールはこの骨格を
//! 最小の形で写し取ったもの。コアモジュール(`fs` 等)・`package.json` の
//! `exports` フィールド・シンボリックリンク解決などは v0.1.0 では未対応。

use std::collections::BTreeSet;

/// 解決の途中でファイル/ディレクトリの存在や `package.json` の `main`
/// フィールドを問い合わせるための抽象。実ファイルシステムを叩く実装も、
/// テスト用のインメモリ実装([`MapFileSystem`])も、このトレイトを介して
/// 同じ解決ロジックに差し込める。
///
/// パスはすべて POSIX 風のスラッシュ区切りの絶対パス相当(`/` 始まり)で
/// 扱う。OS 依存のパス表現には踏み込まない(純粋なモデルとしての解決規則を
/// 検証することが目的のため)。
pub trait FileSystem {
    /// `path` が「ファイルとして存在する」なら true。
    fn is_file(&self, path: &str) -> bool;

    /// `path` が「ディレクトリとして存在する」なら true。
    fn is_dir(&self, path: &str) -> bool;

    /// `dir/package.json` の `main` フィールドの値を返す(無ければ `None`)。
    /// 返り値は `dir` からの相対パス、または絶対パス相当の文字列。
    fn package_main(&self, dir: &str) -> Option<String>;
}

/// 解決失敗の理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// どの候補パスにも解決できなかった(Node の `MODULE_NOT_FOUND` 相当)。
    NotFound {
        request: String,
        from_dir: String,
    },
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::NotFound { request, from_dir } => write!(
                f,
                "Cannot find module '{request}' from '{from_dir}'"
            ),
        }
    }
}

impl std::error::Error for ResolveError {}

/// 補完を試みる拡張子。Node の既定に倣い `.js` → `.json` の順。
const EXTENSIONS: &[&str] = &[".js", ".json"];

/// `require(request)` を `from_dir` を基準に解決し、解決先の絶対パス相当を返す。
///
/// - `request` が `./` `../` `/` で始まる → 相対/絶対パスとして
///   `LOAD_AS_FILE` → `LOAD_AS_DIRECTORY` を試みる。
/// - それ以外(ベア指定、例 `lodash`)→ `from_dir` から親方向へ
///   `node_modules` を辿って探索する(`LOAD_NODE_MODULES`)。
pub fn resolve(
    request: &str,
    from_dir: &str,
    fs: &dyn FileSystem,
) -> Result<String, ResolveError> {
    let not_found = || ResolveError::NotFound {
        request: request.to_string(),
        from_dir: from_dir.to_string(),
    };

    if request.starts_with("./") || request.starts_with("../") || request.starts_with('/') {
        let base = if let Some(stripped) = request.strip_prefix('/') {
            // 絶対パス指定。先頭 `/` を保ったまま正規化する。
            normalize(&format!("/{stripped}"))
        } else {
            normalize(&join(from_dir, request))
        };

        load_as_file(&base, fs)
            .or_else(|| load_as_directory(&base, fs))
            .ok_or_else(not_found)
    } else {
        // ベア指定: node_modules 探索。
        for modules_dir in node_modules_paths(from_dir) {
            let candidate = normalize(&join(&modules_dir, request));
            if let Some(hit) =
                load_as_file(&candidate, fs).or_else(|| load_as_directory(&candidate, fs))
            {
                return Ok(hit);
            }
        }
        Err(not_found())
    }
}

/// `LOAD_AS_FILE(X)`: `X` そのもの → `X.js` → `X.json` の順に試す。
fn load_as_file(path: &str, fs: &dyn FileSystem) -> Option<String> {
    if fs.is_file(path) {
        return Some(path.to_string());
    }
    for ext in EXTENSIONS {
        let candidate = format!("{path}{ext}");
        if fs.is_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

/// `LOAD_AS_DIRECTORY(X)`:
/// 1. `X/package.json` の `main` を解決先の起点にする(ファイル補完付き)。
/// 2. `main` が無い/解決できなければ `X/index.js` → `X/index.json`。
fn load_as_directory(path: &str, fs: &dyn FileSystem) -> Option<String> {
    if !fs.is_dir(path) {
        return None;
    }

    if let Some(main) = fs.package_main(path) {
        let main_path = normalize(&join(path, &main));
        if let Some(hit) = load_as_file(&main_path, fs) {
            return Some(hit);
        }
        // main がディレクトリを指す場合、その index を試す。
        if let Some(hit) = load_index(&main_path, fs) {
            return Some(hit);
        }
    }

    load_index(path, fs)
}

/// `X/index.js` → `X/index.json`。
fn load_index(dir: &str, fs: &dyn FileSystem) -> Option<String> {
    for ext in EXTENSIONS {
        let candidate = normalize(&join(dir, &format!("index{ext}")));
        if fs.is_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

/// `NODE_MODULES_PATHS(start)`: `start` から根まで親方向へ辿りつつ、
/// 各階層の `node_modules` を候補列として返す(近い階層が先)。
/// 既に `node_modules` 内にいる階層は Node と同様にスキップする。
pub fn node_modules_paths(start: &str) -> Vec<String> {
    let normalized = normalize(start);
    let mut parts: Vec<&str> = normalized.split('/').filter(|s| !s.is_empty()).collect();
    let mut result = Vec::new();
    loop {
        if parts.last() != Some(&"node_modules") {
            let mut dir = String::from("/");
            dir.push_str(&parts.join("/"));
            if dir == "/" {
                result.push("/node_modules".to_string());
            } else {
                result.push(format!("{dir}/node_modules"));
            }
        }
        if parts.is_empty() {
            break;
        }
        parts.pop();
    }
    result
}

/// 2 つのパスを結合する(`base` が絶対、`rel` が相対または絶対)。
/// `rel` が `/` 始まりなら `rel` をそのまま採用する。
fn join(base: &str, rel: &str) -> String {
    if rel.starts_with('/') {
        rel.to_string()
    } else {
        format!("{}/{}", base.trim_end_matches('/'), rel)
    }
}

/// POSIX 風パスの正規化: `.` を除去、`..` を親へ畳み込み、重複スラッシュを
/// 圧縮する。先頭が `/` なら絶対パスとして保つ。
pub fn normalize(path: &str) -> String {
    let is_absolute = path.starts_with('/');
    let mut stack: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if let Some(&last) = stack.last() {
                    if last != ".." {
                        stack.pop();
                        continue;
                    }
                }
                if !is_absolute {
                    stack.push("..");
                }
                // 絶対パスで根を超える `..` は捨てる(Node と同様)。
            }
            other => stack.push(other),
        }
    }
    let joined = stack.join("/");
    if is_absolute {
        format!("/{joined}")
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}

/// テスト・実験用のインメモリ [`FileSystem`]。存在するファイル/ディレクトリの
/// 集合と、`package.json` の `main` フィールドのマップを保持する。
#[derive(Debug, Default, Clone)]
pub struct MapFileSystem {
    files: BTreeSet<String>,
    dirs: BTreeSet<String>,
    package_mains: std::collections::BTreeMap<String, String>,
}

impl MapFileSystem {
    /// 空のファイルシステムを作る。
    pub fn new() -> Self {
        Self::default()
    }

    /// ファイルを 1 つ登録し、その全祖先ディレクトリも自動で登録する。
    pub fn with_file(mut self, path: &str) -> Self {
        let path = normalize(path);
        // 祖先ディレクトリを登録。
        let mut parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        parts.pop(); // ファイル名を除く
        let mut acc = String::new();
        for p in parts {
            acc.push('/');
            acc.push_str(p);
            self.dirs.insert(acc.clone());
        }
        self.dirs.insert("/".to_string());
        self.files.insert(path);
        self
    }

    /// `dir/package.json` の `main` フィールドを登録する。
    /// `dir` 自体はディレクトリとして登録される。
    pub fn with_package_main(mut self, dir: &str, main: &str) -> Self {
        let dir = normalize(dir);
        self.dirs.insert(dir.clone());
        self.package_mains.insert(dir, main.to_string());
        self
    }
}

impl FileSystem for MapFileSystem {
    fn is_file(&self, path: &str) -> bool {
        self.files.contains(&normalize(path))
    }

    fn is_dir(&self, path: &str) -> bool {
        self.dirs.contains(&normalize(path))
    }

    fn package_main(&self, dir: &str) -> Option<String> {
        self.package_mains.get(&normalize(dir)).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_collapses_dot_and_dotdot() {
        assert_eq!(normalize("/a/b/../c"), "/a/c");
        assert_eq!(normalize("/a/./b/"), "/a/b");
        assert_eq!(normalize("/a//b"), "/a/b");
        assert_eq!(normalize("/a/b/../../.."), "/");
        assert_eq!(normalize("a/b/../c"), "a/c");
    }

    #[test]
    fn resolve_relative_exact_file() {
        let fs = MapFileSystem::new().with_file("/app/lib/util.js");
        let got = resolve("./lib/util.js", "/app", &fs).unwrap();
        assert_eq!(got, "/app/lib/util.js");
    }

    #[test]
    fn resolve_relative_extension_completion_js() {
        let fs = MapFileSystem::new().with_file("/app/lib/util.js");
        // 拡張子なしで要求 → .js 補完。
        let got = resolve("./lib/util", "/app", &fs).unwrap();
        assert_eq!(got, "/app/lib/util.js");
    }

    #[test]
    fn resolve_extension_prefers_js_over_json() {
        let fs = MapFileSystem::new()
            .with_file("/app/data.js")
            .with_file("/app/data.json");
        let got = resolve("./data", "/app", &fs).unwrap();
        assert_eq!(got, "/app/data.js", ".js が .json より優先される");
    }

    #[test]
    fn resolve_json_when_only_json_exists() {
        let fs = MapFileSystem::new().with_file("/app/config.json");
        let got = resolve("./config", "/app", &fs).unwrap();
        assert_eq!(got, "/app/config.json");
    }

    #[test]
    fn resolve_directory_index() {
        let fs = MapFileSystem::new().with_file("/app/widgets/index.js");
        let got = resolve("./widgets", "/app", &fs).unwrap();
        assert_eq!(got, "/app/widgets/index.js");
    }

    #[test]
    fn resolve_directory_package_main() {
        let fs = MapFileSystem::new()
            .with_package_main("/app/mylib", "./src/entry.js")
            .with_file("/app/mylib/src/entry.js");
        let got = resolve("./mylib", "/app", &fs).unwrap();
        assert_eq!(got, "/app/mylib/src/entry.js");
    }

    #[test]
    fn resolve_package_main_falls_back_to_index() {
        // main が壊れている場合は index.js にフォールバック。
        let fs = MapFileSystem::new()
            .with_package_main("/app/mylib", "./does-not-exist.js")
            .with_file("/app/mylib/index.js");
        let got = resolve("./mylib", "/app", &fs).unwrap();
        assert_eq!(got, "/app/mylib/index.js");
    }

    #[test]
    fn resolve_parent_relative() {
        let fs = MapFileSystem::new().with_file("/app/shared/const.js");
        let got = resolve("../shared/const", "/app/pages", &fs).unwrap();
        assert_eq!(got, "/app/shared/const.js");
    }

    #[test]
    fn resolve_absolute_request() {
        let fs = MapFileSystem::new().with_file("/etc/app/boot.js");
        let got = resolve("/etc/app/boot", "/anywhere", &fs).unwrap();
        assert_eq!(got, "/etc/app/boot.js");
    }

    #[test]
    fn resolve_bare_from_node_modules() {
        let fs = MapFileSystem::new().with_file("/app/node_modules/lodash/index.js");
        let got = resolve("lodash", "/app/src", &fs).unwrap();
        assert_eq!(got, "/app/node_modules/lodash/index.js");
    }

    #[test]
    fn resolve_bare_prefers_nearest_node_modules() {
        let fs = MapFileSystem::new()
            .with_file("/app/node_modules/dep/index.js")
            .with_file("/app/src/node_modules/dep/index.js");
        // /app/src から探すと、より近い src/node_modules が優先される。
        let got = resolve("dep", "/app/src", &fs).unwrap();
        assert_eq!(got, "/app/src/node_modules/dep/index.js");
    }

    #[test]
    fn resolve_bare_walks_up_to_ancestor() {
        let fs = MapFileSystem::new().with_file("/node_modules/glob/index.js");
        let got = resolve("glob", "/app/src/deep", &fs).unwrap();
        assert_eq!(got, "/node_modules/glob/index.js");
    }

    #[test]
    fn resolve_bare_with_subpath() {
        let fs = MapFileSystem::new().with_file("/app/node_modules/lodash/fp/map.js");
        let got = resolve("lodash/fp/map", "/app", &fs).unwrap();
        assert_eq!(got, "/app/node_modules/lodash/fp/map.js");
    }

    #[test]
    fn resolve_not_found_errors() {
        let fs = MapFileSystem::new().with_file("/app/other.js");
        let err = resolve("./missing", "/app", &fs).unwrap_err();
        assert!(matches!(err, ResolveError::NotFound { .. }));
    }

    #[test]
    fn node_modules_paths_skips_existing_node_modules() {
        let paths = node_modules_paths("/app/node_modules/pkg");
        // pkg 直下、app、根 の node_modules は含むが
        // 「.../node_modules/node_modules」は生成しない。
        assert!(paths.contains(&"/app/node_modules/pkg/node_modules".to_string()));
        assert!(paths.contains(&"/app/node_modules".to_string()));
        assert!(paths.contains(&"/node_modules".to_string()));
        assert!(!paths.iter().any(|p| p.ends_with("node_modules/node_modules")));
    }
}
