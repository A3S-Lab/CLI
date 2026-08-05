use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum IdeIntelligenceCommand {
    Status,
    Symbols { query: Option<String> },
    Navigate(NavigationKind),
    Diagnostics { workspace: bool },
}

pub(super) fn parse_ide_intelligence_command(
    command: &str,
) -> Option<Result<IdeIntelligenceCommand, String>> {
    let command = command.trim();
    let split_at = command.find(char::is_whitespace).unwrap_or(command.len());
    let name = &command[..split_at];
    let argument = command[split_at..].trim();

    let no_argument = |label: &str| {
        if argument.is_empty() {
            Ok(())
        } else {
            Err(format!("{label} does not accept arguments"))
        }
    };

    match name {
        "status" => Some(no_argument(":status").map(|()| IdeIntelligenceCommand::Status)),
        "symbols" => Some(if argument.is_empty() {
            Ok(IdeIntelligenceCommand::Symbols { query: None })
        } else if argument.chars().count() > IDE_INTELLIGENCE_QUERY_MAX_CHARS {
            Err(format!(
                ":symbols query is too long; maximum {IDE_INTELLIGENCE_QUERY_MAX_CHARS} characters"
            ))
        } else {
            let query = sanitize_ide_intelligence_field(argument, IDE_INTELLIGENCE_QUERY_MAX_CHARS);
            if query.is_empty() {
                Err(":symbols query must contain visible text".to_owned())
            } else {
                Ok(IdeIntelligenceCommand::Symbols { query: Some(query) })
            }
        }),
        "definition" => Some(
            no_argument(":definition")
                .map(|()| IdeIntelligenceCommand::Navigate(NavigationKind::Definition)),
        ),
        "declaration" => Some(
            no_argument(":declaration")
                .map(|()| IdeIntelligenceCommand::Navigate(NavigationKind::Declaration)),
        ),
        "references" => Some(
            no_argument(":references")
                .map(|()| IdeIntelligenceCommand::Navigate(NavigationKind::References)),
        ),
        "implementations" => Some(
            no_argument(":implementations")
                .map(|()| IdeIntelligenceCommand::Navigate(NavigationKind::Implementations)),
        ),
        "diagnostics" => Some(match argument {
            "" => Ok(IdeIntelligenceCommand::Diagnostics { workspace: false }),
            "workspace" => Ok(IdeIntelligenceCommand::Diagnostics { workspace: true }),
            _ => Err("usage: :diagnostics [workspace]".to_owned()),
        }),
        _ => None,
    }
}
