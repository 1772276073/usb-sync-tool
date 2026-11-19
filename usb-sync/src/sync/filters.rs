//! 排除规则：简单 glob 匹配（沿用旧版 sync-filters.txt 语义，并修正锚定问题）
//!
//! 规则语法：
//! - 不含 `/` 的模式：按文件/目录名匹配任意层级（如 `*.tmp`、`node_modules`）。
//!   匹配到目录名时整棵子树跳过。
//! - 含 `/` 的模式：锚定在同步根的相对路径上（如 `/node_modules/**`）。
//!   以 `/**` 结尾表示"该目录及全部内容"；以 `**/` 开头表示"任意层级下的"（退化为名称规则）。
//! - 通配符：`*`（不跨目录段）、`**`（跨任意层级）、`?`（单个字符）。匹配不区分大小写。

/// 默认排除规则（内部目录强制锚定根目录，修复旧版 `rclone/**` 不锚定的误伤）
pub const DEFAULT_FILTERS: &[&str] = &[
    ".DS_Store",
    "Thumbs.db",
    "desktop.ini",
    "*.tmp",
    "*.temp",
    "*.usb-partial",
    "~$*",
    "/.sync-backup/**",
    "/.usb-sync/**",
];

/// 内部目录保护规则（始终追加在用户规则之后，防止状态/备份目录被同步自身）
pub const INTERNAL_FILTERS: &[&str] = &["/.sync-backup/**", "/.usb-sync/**"];

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ch(char),
    Any,      // *   不跨 /
    AnyDeep,  // **  跨任意层级
    Qmark,    // ?   单个非 /
}

#[derive(Debug, Clone)]
struct PathRule {
    /// 完整锚定模式（相对路径 glob）
    pattern: String,
    /// 若以 `/**` 结尾，为去掉该后缀的目录前缀（用于扫描时剪枝整棵子树）
    dir_prefix: Option<String>,
    tok: Vec<Tok>,
}

#[derive(Debug, Clone, Default)]
pub struct Filters {
    name_rules: Vec<Vec<Tok>>,
    path_rules: Vec<PathRule>,
}

impl Filters {
    pub fn new(rules: &[String]) -> Filters {
        let mut f = Filters::default();
        for raw in rules {
            let rule = raw.trim();
            if rule.is_empty() || rule.starts_with('#') {
                continue;
            }
            // **/ 前缀 → 退化为名称规则
            let rule = rule.strip_prefix("**/").unwrap_or(rule);
            if rule.contains('/') {
                let pattern = rule.trim_start_matches('/');
                let dir_prefix = pattern.strip_suffix("/**").map(|p| p.trim_end_matches('/').to_string());
                let mut pat = pattern.to_string();
                // `dir/**` 同时也排除 dir 空目录本身
                if dir_prefix.is_some() {
                    // 保留原模式用于文件匹配；目录剪枝用 dir_prefix
                } else if let Some(stripped) = pattern.strip_suffix('/') {
                    pat = stripped.to_string();
                }
                if let Some(tok) = tokenize(&pat) {
                    f.path_rules.push(PathRule { pattern: pat, dir_prefix, tok });
                }
            } else if let Some(tok) = tokenize(rule) {
                f.name_rules.push(tok);
            }
        }
        f
    }

    /// 用户规则 + 默认规则 + 内部保护
    pub fn with_defaults(user: &[String]) -> Filters {
        let mut all: Vec<String> = user.to_vec();
        for d in DEFAULT_FILTERS {
            all.push(d.to_string());
        }
        for d in INTERNAL_FILTERS {
            all.push(d.to_string());
        }
        Filters::new(&all)
    }

    /// 文件是否被排除
    pub fn excluded_file(&self, rel: &str) -> bool {
        let rel_lc = rel.to_lowercase();
        let last_segment = rel_lc.rsplit('/').next().unwrap_or("");
        let name_chars: Vec<char> = last_segment.chars().collect();
        for tok in &self.name_rules {
            if glob_match(tok, &name_chars) {
                return true;
            }
        }
        for rule in &self.path_rules {
            let chars: Vec<char> = rel_lc.chars().collect();
            if glob_match(&rule.tok, &chars) {
                return true;
            }
            // 目录前缀规则：子路径也排除（扫描剪枝兜底）
            if let Some(prefix) = &rule.dir_prefix {
                let p = prefix.to_lowercase();
                if rel_lc == p || rel_lc.starts_with(&format!("{}/", p)) {
                    return true;
                }
            }
        }
        false
    }

    /// 目录是否被排除（扫描时直接剪枝，不进入子树）
    pub fn excluded_dir(&self, rel: &str, name: &str) -> bool {
        let name_lc = name.to_lowercase();
        for tok in &self.name_rules {
            if glob_match(tok, &name_lc.chars().collect::<Vec<char>>()) {
                return true;
            }
        }
        let rel_lc = rel.to_lowercase();
        for rule in &self.path_rules {
            if let Some(prefix) = &rule.dir_prefix {
                let p = prefix.to_lowercase();
                if rel_lc == p || rel_lc.starts_with(&format!("{}/", p)) {
                    return true;
                }
                // 前缀本身含 glob 时（如 /build-*/），模式匹配目录路径
                let chars: Vec<char> = rel_lc.chars().collect();
                if glob_match(&rule.tok, &chars) && rule.pattern.ends_with("/**") {
                    return true;
                }
            } else {
                let chars: Vec<char> = rel_lc.chars().collect();
                if glob_match(&rule.tok, &chars) {
                    return true;
                }
            }
        }
        false
    }
}

fn tokenize(pattern: &str) -> Option<Vec<Tok>> {
    let mut toks = Vec::new();
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    toks.push(Tok::AnyDeep);
                } else {
                    toks.push(Tok::Any);
                }
            }
            '/' => {
                // `dir/**`：斜杠后紧跟 `**` → 目录前缀跨层通配
                if chars.peek() == Some(&'*') {
                    chars.next(); // 第一个 *
                    if chars.peek() == Some(&'*') {
                        chars.next(); // 第二个 *
                        if chars.peek() == Some(&'/') {
                            chars.next();
                        }
                        toks.push(Tok::AnyDeep);
                    } else {
                        toks.push(Tok::Ch('/'));
                        toks.push(Tok::Any);
                    }
                } else {
                    toks.push(Tok::Ch('/'));
                }
            }
            '?' => toks.push(Tok::Qmark),
            other => toks.push(Tok::Ch(other.to_ascii_lowercase())),
        }
    }
    if toks.is_empty() {
        None
    } else {
        Some(toks)
    }
}

/// 迭代式 glob 匹配（带 `*`/`**` 回溯），输入需已按模式同口径小写化
fn glob_match(toks: &[Tok], text: &[char]) -> bool {
    match toks.split_first() {
        None => text.is_empty(),
        Some((Tok::Ch(c), rest)) => {
            matches!(text.split_first(), Some((t, tr)) if t.eq_ignore_ascii_case(c) && glob_match(rest, tr))
        }
        Some((Tok::Qmark, rest)) => {
            matches!(text.split_first(), Some((t, tr)) if *t != '/' && glob_match(rest, tr))
        }
        Some((Tok::Any, rest)) => (0..=text.len())
            .take_while(|&i| i == 0 || text[i - 1] != '/')
            .any(|i| glob_match(rest, &text[i..])),
        Some((Tok::AnyDeep, rest)) => (0..=text.len()).any(|i| glob_match(rest, &text[i..])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(rules: &[&str]) -> Filters {
        Filters::new(&rules.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn name_glob_basics() {
        let flt = f(&["*.tmp"]);
        assert!(flt.excluded_file("a.tmp"));
        assert!(flt.excluded_file("sub/dir/b.TMP")); // 大小写不敏感 + 任意层级
        assert!(!flt.excluded_file("a.tmpx"));
        assert!(!flt.excluded_file("dir/a.txt"));
    }

    #[test]
    fn name_rules_match_any_depth() {
        let flt = f(&[".DS_Store", "Thumbs.db", "~$*"]);
        assert!(flt.excluded_file("deep/nest/.DS_Store"));
        assert!(flt.excluded_file("Thumbs.db"));
        assert!(flt.excluded_file("docs/~$report.docx"));
        assert!(!flt.excluded_file("docs/report.docx"));
    }

    #[test]
    fn dir_name_prunes_subtree() {
        let flt = f(&["node_modules"]);
        assert!(flt.excluded_dir("proj/node_modules", "node_modules"));
        // 文件按名称规则同样命中
        assert!(flt.excluded_file("x/node_modules"));
    }

    #[test]
    fn anchored_path_rules() {
        let flt = f(&["/.sync-backup/**", "/build-*/**", "/keep/root.txt"]);
        assert!(flt.excluded_file(".sync-backup/run1/a.txt"));
        assert!(flt.excluded_dir(".sync-backup", ".sync-backup"));
        // 其他层级同名目录不受影响（锚定语义）
        assert!(!flt.excluded_file("other/.sync-backup/a.txt"));
        assert!(flt.excluded_file("build-2026/out.js"));
        assert!(flt.excluded_dir("build-2026", "build-2026"));
        assert!(!flt.excluded_file("src/build-2026/out.js")); // 锚定根，不误伤
        assert!(flt.excluded_file("keep/root.txt"));
        assert!(!flt.excluded_file("other/root.txt"));
    }

    #[test]
    fn glob_matching_semantics() {
        let star = tokenize("*.log").unwrap();
        assert!(glob_match(&star, &"a.log".chars().collect::<Vec<_>>()));
        assert!(!glob_match(&star, &"a/b.log".chars().collect::<Vec<_>>()));
        let deep = tokenize("node_modules/**").unwrap();
        assert!(glob_match(&deep, &"node_modules/pkg/i.js".chars().collect::<Vec<_>>()));
        assert!(glob_match(&deep, &"node_modules".chars().collect::<Vec<_>>()));
        let q = tokenize("file?.txt").unwrap();
        assert!(glob_match(&q, &"file1.txt".chars().collect::<Vec<_>>()));
        assert!(!glob_match(&q, &"file12.txt".chars().collect::<Vec<_>>()));
    }

    #[test]
    fn leading_double_star_degrades_to_name_rule() {
        let flt = f(&["**/.git"]);
        assert!(flt.excluded_dir("a/b/.git", ".git"));
        assert!(flt.excluded_file(".git"));
    }

    #[test]
    fn comments_and_empty_ignored() {
        let flt = f(&["# 注释", "", "  ", "*.tmp"]);
        assert!(flt.excluded_file("a.tmp"));
        assert_eq!(flt.name_rules.len(), 1);
    }

    #[test]
    fn internal_dirs_always_excluded() {
        let flt = Filters::with_defaults(&[]);
        assert!(flt.excluded_file(".usb-sync/state.json"));
        assert!(flt.excluded_file(".sync-backup/2026/a.txt"));
        assert!(flt.excluded_dir(".usb-sync", ".usb-sync"));
    }
}
