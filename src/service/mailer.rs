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
        println!("{:?}", cfg);
        send_verification_code(&cfg, "mail@example.com", "Your email verification code is: AFKC-ADI2").await.unwrap();
    }
}