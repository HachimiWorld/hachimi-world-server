use lettre::message::header::ContentType;
use lettre::message::Mailbox;
use lettre::{SmtpTransport, Transport};
use lettre::transport::smtp::authentication::Credentials;
use crate::web::routes::auth::EmailConfig;

pub async fn send_verification_code(
    cfg: &EmailConfig,
    to: &str,
    code: &str
) -> anyhow::Result<()> {
    let email_msg = lettre::Message::builder()
        .from(Mailbox::new(
            Some("Hachimi World".to_string()),
            cfg.no_reply_email.parse()?,
        ))
        .to(Mailbox::new(None, to.parse()?))
        .subject("Your email verification code - Hachimi World")
        .header(ContentType::TEXT_PLAIN)
        .body(code.to_string())?;

    let creds = Credentials::new(cfg.username.clone(), cfg.password.clone());

    let mailer = SmtpTransport::relay(cfg.host.as_str())?
        .credentials(creds)
        .build();
    mailer.send(&email_msg)?;
    Ok(())
}