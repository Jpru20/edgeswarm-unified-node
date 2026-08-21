use std::env;

fn configured_value(primary: &str, secondary: &str, compiled: Option<&'static str>, missing: &str) -> Result<String, String> {
    env::var(primary).ok().or_else(|| env::var(secondary).ok()).or_else(|| compiled.map(str::to_string)).map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).ok_or_else(|| missing.to_string())
}

pub fn supabase_url_v1() -> Result<String, String> {
    configured_value("SUPABASE_URL", "EDGESWARM_SUPABASE_URL", option_env!("EDGESWARM_DEFAULT_SUPABASE_URL"), "supabase_url_missing").map(|v| v.trim_end_matches('/').to_string())
}

pub fn supabase_anon_key_v1() -> Result<String, String> {
    configured_value("SUPABASE_ANON_KEY", "EDGESWARM_SUPABASE_ANON_KEY", option_env!("EDGESWARM_DEFAULT_SUPABASE_ANON_KEY"), "supabase_anon_key_missing")
}
