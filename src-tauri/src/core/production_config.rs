use std::env;

fn configured_value(primary: &str, secondary: &str, compiled: Option<&'static str>, missing: &str) -> Result<String, String> {
    env::var(primary).ok().or_else(|| env::var(secondary).ok()).or_else(|| compiled.map(str::to_string)).map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).ok_or_else(|| missing.to_string())
}


// UNIFIED_PUBLIC_SUPABASE_DEFAULTS_V1
//
// These are public client configuration values, not privileged service
// credentials. Runtime environment variables still take precedence.
const DEFAULT_SUPABASE_URL_V1: &str =
    "https://xrmwmoqgukjztboemvgi.supabase.co";

const DEFAULT_SUPABASE_ANON_KEY_V1: &str =
    "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJzdXBhYmFzZSIsInJlZiI6InhybXdtb3FndWtqenRib2VtdmdpIiwicm9sZSI6ImFub24iLCJpYXQiOjE3Nzk3MzgzNDcsImV4cCI6MjA5NTMxNDM0N30.3kP1uRFgRAgr2L2eh3Su36icRUHMEsfYIJc1RBV1jjM";

pub fn supabase_url_v1() -> Result<String, String> {
    configured_value("SUPABASE_URL", "EDGESWARM_SUPABASE_URL", option_env!("EDGESWARM_DEFAULT_SUPABASE_URL").or(Some(DEFAULT_SUPABASE_URL_V1)), "supabase_url_missing").map(|v| v.trim_end_matches('/').to_string())
}

pub fn supabase_anon_key_v1() -> Result<String, String> {
    configured_value("SUPABASE_ANON_KEY", "EDGESWARM_SUPABASE_ANON_KEY", option_env!("EDGESWARM_DEFAULT_SUPABASE_ANON_KEY").or(Some(DEFAULT_SUPABASE_ANON_KEY_V1)), "supabase_anon_key_missing")
}
