//! Grafana Loki and Kibana deep links — ports of `grafanaUrl.ts` and `tasks/KibanaLink.tsx`.

use chrono::{DateTime, Utc};

use crate::config::Env;

const GRAFANA_BASE: &str =
    "https://complyadvantage.grafana.net/a/grafana-lokiexplore-app/explore/environment";

/// Port of `buildGrafanaLogsUrl`.
pub fn grafana_logs_url(
    env: Env,
    recipe_run_id: &str,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
) -> String {
    let genv = env.grafana_env();
    let query = [
        ("var-ds", "grafanacloud-logs".to_string()),
        ("var-filters", format!("environment|=|{genv}")),
        (
            "var-lineFilters",
            format!("caseInsensitive,0|__gfp__=|{recipe_run_id}"),
        ),
        (
            "from",
            started_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        ),
        (
            "to",
            ended_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        ),
        ("displayedFields", r#"["service_name"]"#.to_string()),
        ("userDisplayedFields", "true".to_string()),
    ];

    let encoded = query
        .iter()
        .map(|(k, v)| format!("{}={}", k, form_urlencode(v)))
        .collect::<Vec<_>>()
        .join("&");

    format!("{GRAFANA_BASE}/{genv}/logs?{encoded}")
}

/// Port of the Kibana discover template. The `source_id` comes from the recipe description.
pub fn kibana_discover_url(env: Env, source_id: &str) -> String {
    let base = format!(
        "https://analytics-es-kibana.k8s.euw1.df-{}-1.ivxs.uk",
        env.kibana_env()
    );
    format!(
        "{base}/app/discover#/?_g=(filters:!(),refreshInterval:(pause:!t,value:0),time:(from:now-15m,to:now))\
&_a=(columns:!(data.names.name,meta.document_type_code,sources.source_ids,data.locations.name,data.aml_types.aml_type),\
filters:!(('$state':(store:appState),meta:(alias:!n,disabled:!f,key:source_ids_original,negate:!f,\
params:(query:'{source_id}'),type:phrase),query:(match_phrase:(source_ids_original:'{source_id}'))),\
('$state':(store:appState),meta:(alias:!n,disabled:!f,\
key:meta.document_type_code,negate:!f,params:(query:SM),type:phrase),query:(match_phrase:(meta.document_type_code:SM)))),\
interval:auto,query:(language:lucene,query:''),sort:!())"
    )
}

/// `application/x-www-form-urlencoded`, matching `URLSearchParams` in the UI (spaces as `+`).
fn form_urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => out.push(*byte as char),
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    /// Same assertions as the UI's grafanaUrl.test.ts.
    #[test]
    fn development_env_appears_in_path_and_filters() {
        let url = grafana_logs_url(
            Env::Development,
            "run-123",
            ts("2026-02-12T11:00:35.202Z"),
            ts("2026-02-12T11:10:35.202Z"),
        );
        assert!(url.contains("/environment/development/logs"), "{url}");
        assert!(
            url.contains("var-filters=environment%7C%3D%7Cdevelopment"),
            "{url}"
        );
    }

    #[test]
    fn production_env_appears_in_path_and_filters() {
        let url = grafana_logs_url(
            Env::Production,
            "run-456",
            ts("2026-02-12T11:00:35.202Z"),
            ts("2026-02-12T11:10:35.202Z"),
        );
        assert!(url.contains("/environment/production/logs"), "{url}");
        assert!(
            url.contains("var-filters=environment%7C%3D%7Cproduction"),
            "{url}"
        );
    }

    #[test]
    fn staging_and_local_are_grafana_development() {
        for env in [Env::Local, Env::Staging] {
            let url = grafana_logs_url(
                env,
                "r",
                ts("2026-01-01T00:00:00Z"),
                ts("2026-01-01T00:01:00Z"),
            );
            assert!(
                url.contains("/environment/development/logs"),
                "{env}: {url}"
            );
        }
    }

    #[test]
    fn run_id_is_in_the_line_filter() {
        let url = grafana_logs_url(
            Env::Development,
            "run-123",
            ts("2026-01-01T00:00:00Z"),
            ts("2026-01-01T00:01:00Z"),
        );
        assert!(url.contains("run-123"), "{url}");
        assert!(url.contains("from=2026-01-01T00%3A00%3A00.000Z"), "{url}");
    }

    #[test]
    fn kibana_host_differs_between_staging_and_production() {
        assert!(kibana_discover_url(Env::Development, "S:ABC").contains("df-staging-1"));
        assert!(kibana_discover_url(Env::Production, "S:ABC").contains("df-production-1"));
        assert!(kibana_discover_url(Env::Production, "S:ABC").contains("S:ABC"));
    }
}
