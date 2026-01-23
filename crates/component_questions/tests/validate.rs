use serde_json::json;

use questions::spec::{QuestionSpecItem, QuestionsSpec};

#[test]
fn validate_required_and_regex() {
    let spec = QuestionsSpec {
        id: "webex-setup".to_string(),
        title: "Webex provider setup".to_string(),
        questions: vec![
            QuestionSpecItem {
                name: "webhook_base_url".to_string(),
                title: "Public base URL".to_string(),
                kind: questions::spec::QuestionKind::String,
                required: true,
                default: None,
                help: None,
                choices: vec![],
                validate: Some(questions::spec::QuestionValidate {
                    regex: Some("^https://".to_string()),
                    min: None,
                    max: None,
                }),
                secret: false,
            },
            QuestionSpecItem {
                name: "bot_token".to_string(),
                title: "Webex bot token".to_string(),
                kind: questions::spec::QuestionKind::String,
                required: true,
                default: None,
                help: None,
                choices: vec![],
                validate: None,
                secret: true,
            },
        ],
    };

    let answers = json!({
        "webhook_base_url": "http://not-https"
    });
    let output =
        questions::validate_answers_for_spec(&spec.questions, answers.as_object().unwrap());
    assert_eq!(output.len(), 2);
    assert!(output.iter().any(|err| err.path == "bot_token"));
    assert!(output.iter().any(|err| err.path == "webhook_base_url"));
}
