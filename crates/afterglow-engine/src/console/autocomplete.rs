use bevy::prelude::*;

use super::{ConsoleCvarValue, ConsoleCvars};

#[derive(Resource, Clone, Debug, Eq, PartialEq)]
pub struct ConsoleAutocompleteRegistry {
    pub endpoints: Vec<String>,
}

impl Default for ConsoleAutocompleteRegistry {
    fn default() -> Self {
        Self {
            endpoints: vec!["127.0.0.1:8820".into()],
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsoleCompletion {
    pub replacement: String,
    pub display: String,
    pub description: String,
}

#[derive(Clone, Copy)]
struct Candidate {
    value: &'static str,
    description: &'static str,
}

const TOP_LEVEL: &[Candidate] = &[
    Candidate {
        value: "connect",
        description: "connect to local or remote server",
    },
    Candidate {
        value: "disconnect",
        description: "disconnect from the active server",
    },
    Candidate {
        value: "server",
        description: "local server controls",
    },
    Candidate {
        value: "net",
        description: "network status and debug controls",
    },
    Candidate {
        value: "stats",
        description: "runtime statistics",
    },
    Candidate {
        value: "cvar",
        description: "console variable access",
    },
    Candidate {
        value: "help",
        description: "show command help",
    },
];

const SERVER: &[Candidate] = &[
    Candidate {
        value: "start",
        description: "start local server",
    },
    Candidate {
        value: "stop",
        description: "stop local server",
    },
    Candidate {
        value: "status",
        description: "show local server status",
    },
];

const NET: &[Candidate] = &[
    Candidate {
        value: "status",
        description: "show connection status",
    },
    Candidate {
        value: "stats",
        description: "show packet counters",
    },
    Candidate {
        value: "links",
        description: "show known network links",
    },
    Candidate {
        value: "latency",
        description: "set simulated latency",
    },
];

const STATS: &[Candidate] = &[
    Candidate {
        value: "fps",
        description: "show frame-rate stats",
    },
    Candidate {
        value: "systems",
        description: "show system timing stats",
    },
];

const CVAR: &[Candidate] = &[
    Candidate {
        value: "get",
        description: "read a cvar",
    },
    Candidate {
        value: "set",
        description: "write a cvar",
    },
];

pub fn complete_console_input(world: &World, input: &str) -> Vec<ConsoleCompletion> {
    let cvars = world
        .get_resource::<ConsoleCvars>()
        .cloned()
        .unwrap_or_default();
    let autocomplete = world
        .get_resource::<ConsoleAutocompleteRegistry>()
        .cloned()
        .unwrap_or_default();

    complete_console_input_from(&cvars, &autocomplete, input)
}

pub(crate) fn complete_console_input_from(
    cvars: &ConsoleCvars,
    autocomplete: &ConsoleAutocompleteRegistry,
    input: &str,
) -> Vec<ConsoleCompletion> {
    let context = CompletionContext::new(input);
    let endpoints = autocomplete.endpoints.clone();

    match context.path.as_slice() {
        [] => static_candidates(&context, TOP_LEVEL),
        ["connect"] => connect_candidates(&context, endpoints),
        ["server"] => static_candidates(&context, SERVER),
        ["net"] => static_candidates(&context, NET),
        ["net", "latency"] => static_candidates(
            &context,
            &[Candidate {
                value: "--ms",
                description: "latency in milliseconds",
            }],
        ),
        ["net", "latency", "--ms"] => latency_values(&context),
        ["stats"] => static_candidates(&context, STATS),
        ["cvar"] => static_candidates(&context, CVAR),
        ["cvar", "get"] | ["cvar", "set"] => cvar_name_candidates(&context, &cvars),
        ["cvar", "set", name] => cvar_value_candidates(&context, &cvars, name),
        ["help"] => static_candidates(&context, TOP_LEVEL),
        _ => Vec::new(),
    }
}

struct CompletionContext<'a> {
    path: Vec<&'a str>,
    prefix: &'a str,
}

impl<'a> CompletionContext<'a> {
    fn new(input: &'a str) -> Self {
        let tokens = input.split_whitespace().collect::<Vec<_>>();
        if input.chars().last().is_some_and(char::is_whitespace) {
            return Self {
                path: tokens,
                prefix: "",
            };
        }
        let Some((prefix, path)) = tokens.split_last() else {
            return Self {
                path: Vec::new(),
                prefix: "",
            };
        };
        Self {
            path: path.to_vec(),
            prefix,
        }
    }

    fn replacement(&self, value: &str) -> String {
        let mut tokens = self.path.clone();
        tokens.push(value);
        format!("{} ", tokens.join(" "))
    }
}

fn static_candidates(
    context: &CompletionContext<'_>,
    candidates: &[Candidate],
) -> Vec<ConsoleCompletion> {
    candidates
        .iter()
        .filter(|candidate| candidate.value.starts_with(context.prefix))
        .map(|candidate| ConsoleCompletion {
            replacement: context.replacement(candidate.value),
            display: candidate.value.into(),
            description: candidate.description.into(),
        })
        .collect()
}

fn connect_candidates(
    context: &CompletionContext<'_>,
    endpoints: Vec<String>,
) -> Vec<ConsoleCompletion> {
    let mut completions = static_candidates(
        context,
        &[Candidate {
            value: "local",
            description: "start and connect to an in-process server",
        }],
    );
    completions.extend(
        endpoints
            .into_iter()
            .filter(|endpoint| endpoint.starts_with(context.prefix))
            .map(|endpoint| ConsoleCompletion {
                replacement: context.replacement(&endpoint),
                display: endpoint,
                description: "known server endpoint".into(),
            }),
    );
    completions
}

fn cvar_name_candidates(
    context: &CompletionContext<'_>,
    cvars: &ConsoleCvars,
) -> Vec<ConsoleCompletion> {
    cvars
        .names()
        .filter(|name| name.starts_with(context.prefix))
        .map(|name| ConsoleCompletion {
            replacement: context.replacement(name),
            display: name.into(),
            description: cvars
                .get(name)
                .map_or("cvar", |cvar| cvar.description.as_str())
                .into(),
        })
        .collect()
}

fn cvar_value_candidates(
    context: &CompletionContext<'_>,
    cvars: &ConsoleCvars,
    name: &str,
) -> Vec<ConsoleCompletion> {
    let Some(cvar) = cvars.get(name) else {
        return Vec::new();
    };
    let values = match &cvar.value {
        ConsoleCvarValue::Bool(_) => vec!["false".to_string(), "true".to_string()],
        ConsoleCvarValue::I64(value) => unique_values([
            value.to_string(),
            "0".into(),
            "12".into(),
            "30".into(),
            "60".into(),
            "120".into(),
        ]),
        ConsoleCvarValue::F64(value) => {
            unique_values([value.to_string(), "0.0".into(), "1.0".into()])
        }
        ConsoleCvarValue::Text(value) => vec![value.clone()],
    };
    values
        .into_iter()
        .filter(|value| value.starts_with(context.prefix))
        .map(|value| ConsoleCompletion {
            replacement: context.replacement(&value),
            display: value,
            description: "cvar value".into(),
        })
        .collect()
}

fn latency_values(context: &CompletionContext<'_>) -> Vec<ConsoleCompletion> {
    ["0", "50", "100", "150", "250"]
        .into_iter()
        .filter(|value| value.starts_with(context.prefix))
        .map(|value| ConsoleCompletion {
            replacement: context.replacement(value),
            display: value.into(),
            description: "latency in milliseconds".into(),
        })
        .collect()
}

fn unique_values(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut unique = Vec::new();
    for value in values {
        if !unique.contains(&value) {
            unique.push(value);
        }
    }
    unique
}
