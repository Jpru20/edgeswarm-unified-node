use crate::core::model_registry::{
    ModelSpecV1,
    MODEL_REGISTRY_V1,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct DiscoveredModelV1 {
    pub selected_model: &'static str,
    pub capability: &'static str,
    pub tier: u8,
    pub runtime: &'static str,
    pub min_ram_gb: u16,
    pub default_ctx: u32,
    pub default_max_tokens: u32,
    pub experimental: bool,
    pub path: PathBuf,
    pub file_name: String,
    pub matched_pattern: &'static str,
}

fn wildcard_match_ci(pattern: &str, value: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase().into_bytes();
    let value = value.to_ascii_lowercase().into_bytes();

    let mut dp = vec![false; value.len() + 1];
    dp[0] = true;

    for token in pattern {
        if token == b'*' {
            for index in 1..=value.len() {
                dp[index] = dp[index] || dp[index - 1];
            }
        } else {
            for index in (1..=value.len()).rev() {
                dp[index] =
                    dp[index - 1] && token == value[index - 1];
            }

            dp[0] = false;
        }
    }

    dp[value.len()]
}

fn pattern_specificity(pattern: &str) -> usize {
    pattern
        .chars()
        .filter(|character| *character != '*')
        .count()
}

fn best_registry_match(
    file_name: &str,
) -> Option<(&'static ModelSpecV1, &'static str)> {
    let mut best:
        Option<(&'static ModelSpecV1, &'static str, usize)> = None;

    for spec in MODEL_REGISTRY_V1 {
        for pattern in spec.patterns {
            if !wildcard_match_ci(pattern, file_name) {
                continue;
            }

            let specificity = pattern_specificity(pattern);

            let replace = match best {
                None => true,
                Some((_, _, existing_specificity)) => {
                    specificity > existing_specificity
                }
            };

            if replace {
                best = Some((spec, *pattern, specificity));
            }
        }
    }

    best.map(|(spec, pattern, _)| (spec, pattern))
}

fn candidate_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();

    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return files,
    };

    for entry in entries.flatten() {
        let path = entry.path();

        if path.is_file() {
            if path
                .extension()
                .and_then(|value| value.to_str())
                .map(|value| value.eq_ignore_ascii_case("gguf"))
                == Some(true)
            {
                files.push(path);
            }

            continue;
        }

        if !path.is_dir() {
            continue;
        }

        if path
            .file_name()
            .and_then(|value| value.to_str())
            .map(|value| value.eq_ignore_ascii_case("_downloads"))
            == Some(true)
        {
            continue;
        }

        if let Ok(children) = fs::read_dir(&path) {
            for child in children.flatten() {
                let child_path = child.path();

                if child_path.is_file()
                    && child_path
                        .extension()
                        .and_then(|value| value.to_str())
                        .map(|value| value.eq_ignore_ascii_case("gguf"))
                        == Some(true)
                {
                    files.push(child_path);
                }
            }
        }
    }

    files
}

pub fn discover_models(
    root: &Path,
) -> Vec<DiscoveredModelV1> {
    let mut discovered = Vec::new();

    for path in candidate_files(root) {
        let Some(file_name) =
            path.file_name()
                .and_then(|value| value.to_str())
                .map(str::to_owned)
        else {
            continue;
        };

        let Some((spec, matched_pattern)) =
            best_registry_match(&file_name)
        else {
            continue;
        };

        discovered.push(DiscoveredModelV1 {
            selected_model: spec.selected_model,
            capability: spec.capability,
            tier: spec.tier,
            runtime: spec.runtime,
            min_ram_gb: spec.min_ram_gb,
            default_ctx: spec.default_ctx,
            default_max_tokens: spec.default_max_tokens,
            experimental: spec.experimental,
            path,
            file_name,
            matched_pattern,
        });
    }

    discovered.sort_by(|left, right| {
        let left_index = MODEL_REGISTRY_V1
            .iter()
            .position(|spec| {
                spec.selected_model == left.selected_model
            })
            .unwrap_or(usize::MAX);

        let right_index = MODEL_REGISTRY_V1
            .iter()
            .position(|spec| {
                spec.selected_model == right.selected_model
            })
            .unwrap_or(usize::MAX);

        left_index
            .cmp(&right_index)
            .then_with(|| left.path.cmp(&right.path))
    });

    discovered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coder_14b_wins_over_broad_14b_pattern() {
        let (spec, _) = best_registry_match(
            "Qwen2.5-Coder-14B-Instruct-Q4_K_M.gguf"
        )
        .unwrap();

        assert_eq!(
            spec.selected_model,
            "qwen2.5-coder:14b"
        );
    }

    #[test]
    fn regular_14b_maps_to_regular_model() {
        let (spec, _) = best_registry_match(
            "Qwen2.5-14B-Instruct-Q4_K_M.gguf"
        )
        .unwrap();

        assert_eq!(
            spec.selected_model,
            "qwen2.5:14b"
        );
    }

    #[test]
    fn qwen3_a3b_filename_matches_30b_registry() {
        let (spec, _) = best_registry_match(
            "Qwen_Qwen3-30B-A3B-Instruct-2507-Q4_K_M.gguf"
        )
        .unwrap();

        assert_eq!(
            spec.selected_model,
            "qwen3:30b"
        );
    }
}
