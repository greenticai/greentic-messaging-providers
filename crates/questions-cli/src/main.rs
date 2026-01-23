use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::fs;
use std::io::{self, Read, Write};

#[derive(Debug, Deserialize)]
struct QuestionsSpec {
    id: String,
    title: String,
    questions: Vec<Question>,
}

#[derive(Debug, Deserialize)]
struct Question {
    name: String,
    title: String,
    kind: String,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    default: Option<Value>,
    #[serde(default)]
    help: Option<String>,
    #[serde(default)]
    secret: bool,
}

fn main() -> Result<()> {
    let spec_json = read_spec_json()?;
    let spec: QuestionsSpec = serde_json::from_str(&spec_json)?;
    let answers = ask_questions(&spec)?;
    println!("{}", serde_json::to_string_pretty(&Value::Object(answers))?);
    Ok(())
}

fn read_spec_json() -> Result<String> {
    let mut args = std::env::args().skip(1);
    let mut path = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--spec" => {
                path = args.next();
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            _ => return Err(anyhow!("unknown argument {arg}")),
        }
    }

    if let Some(path) = path {
        return fs::read_to_string(path).context("read spec file");
    }

    let mut buf = String::new();
    io::stdin().read_to_string(&mut buf)?;
    if buf.trim().is_empty() {
        return Err(anyhow!("spec JSON required via --spec or stdin"));
    }
    Ok(buf)
}

fn ask_questions(spec: &QuestionsSpec) -> Result<Map<String, Value>> {
    let mut out = Map::new();
    let mut stdout = io::stdout();
    writeln!(stdout, "{} ({})", spec.title, spec.id).ok();
    for q in &spec.questions {
        let prompt = format_prompt(q);
        let value = loop {
            let raw = if q.secret {
                rpassword::prompt_password(&prompt)?
            } else {
                write!(stdout, "{prompt}")?;
                stdout.flush()?;
                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                input.trim_end().to_string()
            };

            if raw.is_empty() {
                if let Some(default) = q.default.clone() {
                    break default;
                }
                if q.required {
                    continue;
                }
                break Value::String(String::new());
            }

            match parse_answer(&raw, &q.kind) {
                Ok(val) => break val,
                Err(_) => {
                    writeln!(stdout, "Invalid value for {}", q.name).ok();
                    continue;
                }
            }
        };
        out.insert(q.name.clone(), value);
    }
    Ok(out)
}

fn parse_answer(raw: &str, kind: &str) -> Result<Value> {
    match kind {
        "bool" | "boolean" => Ok(Value::Bool(matches!(
            raw.to_ascii_lowercase().as_str(),
            "true" | "1" | "yes" | "y"
        ))),
        "number" | "int" | "integer" => Ok(Value::Number(raw.parse::<i64>()?.into())),
        _ => Ok(Value::String(raw.to_string())),
    }
}

fn format_prompt(question: &Question) -> String {
    let mut prompt = format!(
        "{}{}",
        question.title,
        if question.required { " *" } else { "" }
    );
    if let Some(help) = question.help.as_ref() {
        prompt.push_str(&format!(" [{help}]"));
    }
    prompt.push_str(": ");
    prompt
}

fn print_help() {
    println!("questions-cli --spec <spec.json>");
    println!("If --spec is omitted, reads the QuestionsSpec JSON from stdin.");
}
