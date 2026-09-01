use super::DetailItem;
use crate::model::Service;

pub fn detail_item_value(service: &Service, item: DetailItem) -> String {
    match item {
        DetailItem::Status => service.status.to_string(),
        DetailItem::Source => service.source.to_string(),
        DetailItem::Domain => service.domain.clone(),
        DetailItem::Scope => service.scope.label().to_string(),
        DetailItem::Safety => service.safety_level.to_string(),
        DetailItem::Origin => {
            let mut value = format!(
                "{} — {}",
                service.origin.summary(),
                service.origin.kind.change_hint()
            );
            if let Some(evidence) = service.origin.evidence.first() {
                value.push_str(&format!(" [{evidence}]"));
            }
            value
        }
        DetailItem::Elevation => service.elevation.summary(),
        DetailItem::Plist => service
            .plist_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "-".to_string()),
        DetailItem::BrewFormula => service.brew_formula.as_deref().unwrap_or("-").to_string(),
        DetailItem::BrewStatus => service.brew_status.as_deref().unwrap_or("-").to_string(),
        DetailItem::Command => service.config.command_preview(),
        DetailItem::WorkingDirectory => service
            .config
            .working_directory
            .as_deref()
            .unwrap_or("-")
            .to_string(),
        DetailItem::Stdout => service
            .config
            .stdout_path
            .as_deref()
            .unwrap_or("-")
            .to_string(),
        DetailItem::Stderr => service
            .config
            .stderr_path
            .as_deref()
            .unwrap_or("-")
            .to_string(),
        DetailItem::RunAtLoad => service
            .config
            .run_at_load
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string()),
        DetailItem::KeepAlive => service
            .config
            .keep_alive
            .as_deref()
            .unwrap_or("-")
            .to_string(),
        DetailItem::StartInterval => service
            .config
            .start_interval
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string()),
        DetailItem::Schedule => service.config.schedule_summary(),
        DetailItem::Health => {
            if service.health.is_empty() {
                "clean".to_string()
            } else {
                service.health.join(" | ")
            }
        }
    }
}
