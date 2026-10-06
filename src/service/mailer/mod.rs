use lettre::message::header::{ContentTransferEncoding, ContentType};
use lettre::message::{Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{SmtpTransport, Transport};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailConfig {
    #[serde(default)]
    pub disabled: bool,
    pub host: String,
    pub username: String,
    pub password: String,
    pub no_reply_email: String,
}

const EMAIL_TEMPLATE: &str = include_str!("templates/code_mail_template_zh.html");
const EMAIL_PLAIN_TEMPLATE: &str = include_str!("templates/code_mail_template_zh.txt");
const EMAIL_NOTIFICATION_TEMPLATE: &str = include_str!("templates/general_notification_zh.html");
const SMTP_TIMEOUT: Duration = Duration::from_secs(10);

pub async fn send_verification_code(
    cfg: &EmailConfig,
    to: &str,
    code: &str,
) -> anyhow::Result<()> {
    if cfg.disabled { return Ok(()) }

    let html_content = EMAIL_TEMPLATE.replace("{{VERIFICATION_CODE}}", code);
    let plain_content = EMAIL_PLAIN_TEMPLATE.replace("{{VERIFICATION_CODE}}", code);

    let email_msg = lettre::Message::builder()
        .from(Mailbox::new(
            Some("基米天堂".to_string()),
            cfg.no_reply_email.parse()?,
        ))
        .to(Mailbox::new(None, to.parse()?))
        .subject("请查收你的邮箱验证码")
        .multipart(MultiPart::alternative()
            .singlepart(SinglePart::plain(plain_content))
            .singlepart(SinglePart::builder()
                .header(ContentType::TEXT_HTML)
                .header(ContentTransferEncoding::Base64)
                .body(html_content)
            )
        )?;

    let creds = Credentials::new(cfg.username.clone(), cfg.password.clone());

    let mailer = SmtpTransport::relay(cfg.host.as_str())?
        .credentials(creds)
        .build();
    mailer.send(&email_msg)?;
    Ok(())
}

/// A plain-text email; the HTML part is generated from it when sending.
/// @since 261006
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationEmail {
    pub subject: String,
    pub body: String,
}

/// Blocks until the SMTP server accepts the email or `SMTP_TIMEOUT` passes. Call it off the async
/// runtime, e.g. with `spawn_blocking`.
pub fn send_notification_blocking(cfg: &EmailConfig, to: &str, email: &NotificationEmail) -> anyhow::Result<()> {
    if cfg.disabled { return Ok(()) }

    let html_content = EMAIL_NOTIFICATION_TEMPLATE.replace("{{CONTENT}}", &askama_escape::escape(&email.body, askama_escape::Html).to_string().replace("\n", "<br>"));
    let email_msg = lettre::Message::builder()
        .from(Mailbox::new(
            Some("基米天堂".to_string()),
            cfg.no_reply_email.parse()?,
        ))
        .to(Mailbox::new(None, to.parse()?))
        .subject(email.subject.as_str())
        .multipart(MultiPart::alternative()
            .singlepart(SinglePart::plain(email.body.clone()))
            .singlepart(SinglePart::builder()
                .header(ContentType::TEXT_HTML)
                .header(ContentTransferEncoding::Base64)
                .body(html_content)
            )
        )?;

    let creds = Credentials::new(cfg.username.clone(), cfg.password.clone());

    let mailer = SmtpTransport::relay(cfg.host.as_str())?
        .credentials(creds)
        .timeout(Some(SMTP_TIMEOUT))
        .build();
    mailer.send(&email_msg)?;
    Ok(())
}

pub fn review_approved_email(
    song_display_id: &str,
    song_title: &str,
    user_name: &str,
    comment: Option<&str>,
) -> NotificationEmail {
    NotificationEmail {
        subject: "您提交的作品已通过审核".to_string(),
        body: format!(
            "亲爱的 {user_name}：\n\n您提交的作品《{song_title}》({song_display_id}) 已通过审核。感谢您的投稿！{}",
            comment.map(|c| format!("\n\n审核留言：{c}")).unwrap_or_default()
        ),
    }
}

pub fn review_rejected_email(
    song_display_id: &str,
    song_title: &str,
    user_name: &str,
    comment: &str,
) -> NotificationEmail {
    NotificationEmail {
        subject: "您提交的作品已被退回".to_string(),
        body: format!(
            "亲爱的 {user_name}：\n\n很抱歉，您提交的作品《{song_title}》({song_display_id}) 已被退回。\n\n审核留言：{comment}"
        ),
    }
}

pub fn review_modify_approved_email(
    song_display_id: &str,
    user_name: &str,
    comment: Option<&str>,
) -> NotificationEmail {
    NotificationEmail {
        subject: "您的作品编辑请求已通过".to_string(),
        body: format!(
            "亲爱的 {user_name}：\n\n您的作品编辑请求 ({song_display_id}) 已通过。{}",
            comment.map(|c| format!("\n\n审核留言：{c}")).unwrap_or_default()
        ),
    }
}

pub fn review_modify_rejected_email(
    song_display_id: &str,
    user_name: &str,
    comment: &str,
) -> NotificationEmail {
    NotificationEmail {
        subject: "您的作品编辑请求未通过".to_string(),
        body: format!(
            "亲爱的 {user_name}：\n\n很抱歉，您的作品编辑请求 ({song_display_id}) 未通过。\n\n审核留言：{comment}"
        ),
    }
}


#[cfg(test)]
mod tests {
    use crate::service::mailer::{review_approved_email, review_rejected_email, send_notification_blocking, send_verification_code, EmailConfig};
    use std::fs;

    #[ignore]
    #[tokio::test]
    async fn test() {
        // Not testable without a real server, ignore this
        let content = fs::read_to_string("../../../.local/config.yaml").unwrap();
        let value = yaml_serde::from_str::<yaml_serde::Value>(content.as_str()).unwrap();
        let cfg: EmailConfig = yaml_serde::from_value(value["email"].clone()).unwrap();
        send_verification_code(&cfg, "mail@example.com", "114514").await.unwrap();
        send_notification_blocking(&cfg, "mail@example.com", &review_approved_email("JM-1111", "哈基哈基2", "我不是神人", Some("非常好听"))).unwrap();
        send_notification_blocking(&cfg, "mail@example.com", &review_rejected_email("JM-1111", "哈基哈基", "我不是神人", "请修改标题")).unwrap();
    }
}