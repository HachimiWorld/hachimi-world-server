use lettre::message::header::{ContentTransferEncoding, ContentType};
use lettre::message::{Mailbox, MultiPart, SinglePart};
use lettre::{SmtpTransport, Transport};
use lettre::transport::smtp::authentication::Credentials;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
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

pub async fn send_notification(
    cfg: &EmailConfig,
    to: &str,
    subject: &str,
    content: &str,
) -> anyhow::Result<()> {
    if cfg.disabled { return Ok(()) }

    let html_content = EMAIL_NOTIFICATION_TEMPLATE.replace("{{CONTENT}}", &askama_escape::escape(content, askama_escape::Html).to_string().replace("\n", "<br>"));
    let email_msg = lettre::Message::builder()
        .from(Mailbox::new(
            Some("基米天堂".to_string()),
            cfg.no_reply_email.parse()?,
        ))
        .to(Mailbox::new(None, to.parse()?))
        .subject(subject)
        .multipart(MultiPart::alternative()
            .singlepart(SinglePart::plain(content.to_string()))
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

pub async fn send_review_approved_notification(
    cfg: &EmailConfig,
    to: &str,
    song_display_id: &str,
    song_title: &str,
    user_name: &str,
    comment: Option<&str>
) -> anyhow::Result<()> {
    let content = format!(
        "亲爱的 {user_name}：\n\n您提交的作品《{song_title}》({song_display_id}) 已通过审核。感谢您的投稿！{}",
        comment.map(|c| format!("\n\n审核留言：{c}")).unwrap_or_default()
    );
    send_notification(cfg, to, "您提交的作品已通过审核", &content).await
}

pub async fn send_review_rejected_notification(
    cfg: &EmailConfig,
    to: &str,
    song_display_id: &str,
    song_title: &str,
    user_name: &str,
    comment: &str
) -> anyhow::Result<()> {
    let content = format!(
        "亲爱的 {user_name}：\n\n很抱歉，您提交的作品《{song_title}》({song_display_id}) 已被退回。\n\n审核留言：{comment}"
    );
    send_notification(cfg, to, "您提交的作品已被退回", &content).await
}

#[cfg(test)]
mod test {
    use std::fs;
    use crate::service::mailer::{send_review_approved_notification, send_review_rejected_notification, send_verification_code, EmailConfig};

    #[tokio::test]
    async fn test() {
        let content = fs::read_to_string("config.yaml").unwrap();
        let value = serde_yaml::from_str::<serde_yaml::Value>(content.as_str()).unwrap();
        let cfg: EmailConfig = serde_yaml::from_value(value["email"].clone()).unwrap();
        send_verification_code(&cfg, "mail@example.com", "114514").await.unwrap();
        send_review_approved_notification(&cfg, "mail@example.com", "JM-1111", "哈基哈基2", "我不是神人", Some("非常好听")).await.unwrap();
        send_review_rejected_notification(&cfg, "mail@example.com", "JM-1111", "哈基哈基", "我不是神人", "请修改标题").await.unwrap();
    }
}