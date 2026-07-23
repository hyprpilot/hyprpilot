//! argv flag-detection helpers shared by the per-vendor builders.
//!
//! Every generated flag is suppressed when the captain already spells
//! it — either in the base agent's `args` or the trailing
//! `-- <provider args>`. These pure predicates answer "is this flag /
//! config key already present?" over the combined arg list.

/// Concatenate the base agent args and the trailing provider args into
/// one detection list. Generated flags check against this so a flag
/// authored on either side suppresses the generated duplicate.
pub(super) fn combined_args(base: &[String], provider: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(base.len() + provider.len());
    out.extend_from_slice(base);
    out.extend_from_slice(provider);
    out
}

/// Whether `args` already carries `long` (or its `short` alias), in
/// either `--flag value` or `--flag=value` form.
pub(super) fn has_flag(args: &[String], long: &str, short: Option<&str>) -> bool {
    args.iter().any(|arg| {
        arg == long
            || short.is_some_and(|short| arg == short)
            || arg.strip_prefix(long).is_some_and(|rest| rest.starts_with('='))
    })
}

/// The value that follows `long` (or its `short` alias) in `args`,
/// accepting both `--flag value` and `--flag=value` forms.
pub(super) fn flag_value(args: &[String], long: &str, short: Option<&str>) -> Option<String> {
    for (idx, arg) in args.iter().enumerate() {
        if let Some(value) = arg.strip_prefix(long).and_then(|rest| rest.strip_prefix('=')) {
            return Some(value.to_string());
        }
        if arg == long || short.is_some_and(|short| arg == short) {
            return args.get(idx + 1).cloned();
        }
    }

    None
}

/// Whether `args` carries a codex `-c`/`--config` override for `key`
/// (`-c key=value` or `--config=key=value`).
pub(super) fn has_config_override(args: &[String], key: &str) -> bool {
    args.iter()
        .filter_map(|arg| arg.strip_prefix("--config="))
        .chain(
            args.windows(2)
                .filter_map(|w| matches!(w[0].as_str(), "-c" | "--config").then_some(w[1].as_str())),
        )
        .any(|raw| {
            raw.split_once('=')
                .is_some_and(|(candidate, _)| candidate.trim() == key)
        })
}
