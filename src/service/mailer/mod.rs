use axum::extract::Multipart;
use lettre::message::header::{ContentTransferEncoding, ContentType};
use lettre::message::{Mailbox, MultiPart, SinglePart};
use lettre::{SmtpTransport, Transport};
use lettre::transport::smtp::authentication::Credentials;
use openssl::base64;
use crate::web::routes::auth::EmailConfig;

const EMAIL_TEMPLATE: &str = include_str!("templates/code_mail_template_zh.html");
const EMAIL_PLAIN_TEMPLATE: &str = include_str!("templates/code_mail_template_zh.txt");
pub async fn send_verification_code(
    cfg: &EmailConfig,
    to: &str,
    code: &str,
) -> anyhow::Result<()> {
    let html_content = EMAIL_TEMPLATE.replace("{{VERIFICATION_CODE}}", code);
    let plain_content = EMAIL_PLAIN_TEMPLATE.replace("{{VERIFICATION_CODE}}", code);

    let email_msg = lettre::Message::builder()
        .from(Mailbox::new(
            Some("Hachimi World".to_string()),
            cfg.no_reply_email.parse()?,
        ))
        .to(Mailbox::new(None, to.parse()?))
        .subject("Your email verification code - Hachimi World")
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

pub async fn send_review_approved_notification(
    cfg: &EmailConfig,
    to: &str,
    song_display_id: &str,
    song_title: &str,
    user_name: &str,
    comment: Option<&str>
) -> anyhow::Result<()> {
    // TODO
    Ok(())
}

pub async fn send_review_rejected_notification(
    cfg: &EmailConfig,
    to: &str,
    song_display_id: &str,
    song_title: &str,
    user_name: &str,
    comment: &str
) -> anyhow::Result<()> {
    // TODO
    Ok(())
}

#[cfg(test)]
mod test {
    use std::fs;
    use crate::service::mailer::send_verification_code;
    use crate::web::routes::auth::EmailConfig;

    #[tokio::test]
    async fn test() {
        let content = fs::read_to_string("config.yaml").unwrap();
        let value = serde_yaml::from_str::<serde_yaml::Value>(content.as_str()).unwrap();
        let cfg: EmailConfig = serde_yaml::from_value(value["email"].clone()).unwrap();
        send_verification_code(&cfg, "mail@example.com", "114514").await.unwrap();
    }
}