use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct SystemInfo {
    pub app_version: String,
    pub os_type: String,
    pub os_version: String,
    pub arch: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FeedbackData {
    pub subject: String,
    pub body: String,
    pub include_system_info: bool,
    pub feedback_type: String, // "bug", "feature", "improvement", "other"
}

/// Build the `mailto:` link for a feedback report. The browser opens it: the
/// server is not the reporter's machine, so it must not launch a mail client.
pub async fn send_feedback(feedback: FeedbackData) -> Result<String, String> {
    let mut email_body = feedback.body.clone();

    // Include system information
    if feedback.include_system_info {
        let system_info = get_system_info().await?;
        email_body.push_str("\n\n---\n");
        email_body.push_str("System Information:\n");
        email_body.push_str(&format!("App Version: {}\n", system_info.app_version));
        email_body.push_str(&format!(
            "OS: {} {}\n",
            system_info.os_type, system_info.os_version
        ));
        email_body.push_str(&format!("Architecture: {}\n", system_info.arch));
    }

    // Adjust email subject based on feedback type
    let email_subject = match feedback.feedback_type.as_str() {
        "bug" => format!("[Bug Report] {}", feedback.subject),
        "feature" => format!("[Feature Request] {}", feedback.subject),
        "improvement" => format!("[Improvement] {}", feedback.subject),
        _ => format!("[Feedback] {}", feedback.subject),
    };

    // URL encoding
    let encoded_subject = urlencoding::encode(&email_subject);
    let encoded_body = urlencoding::encode(&email_body);

    // Generate mailto link

    let feedback_email = std::env::var("FEEDBACK_EMAIL")
        .unwrap_or_else(|_| "feedback@claude-history-viewer.app".to_string());
    let mailto_url =
        format!("mailto:{feedback_email}?subject={encoded_subject}&body={encoded_body}");

    Ok(mailto_url)
}

pub async fn get_system_info() -> Result<SystemInfo, String> {
    Ok(SystemInfo {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        os_type: std::env::consts::OS.to_string(),
        os_version: "Unknown".to_string(), // Can be obtained from OS plugin
        arch: std::env::consts::ARCH.to_string(),
    })
}
