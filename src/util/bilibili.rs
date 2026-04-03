use anyhow::anyhow;

#[derive(Debug, Clone)]
pub struct BiliUserProfile {
    pub mid: i64,
    pub name: String,
    pub bio: String,
    pub level: i32
}

pub async fn get_user_profile(mid: i64) -> anyhow::Result<Option<BiliUserProfile>> {
    let handle = tokio::task::spawn(async move {
        let mut cli = bilibili_api_rs::Client::new();
        let mut init_cnt = 0;
        let mut last_error: Option<anyhow::Error> = None;

        let info = loop {
            match cli.user(mid).info().await {
                Ok(v) => break v,
                Err(e) => {
                    if e.to_string().starts_with("bilibili api reject: -404") {
                        return Ok(None)
                    }
                    last_error = Some(anyhow!("failed to get user info: {e}"));
                }
            }
            init_cnt += 1;
            if init_cnt > 5 {
                return Err(last_error.unwrap_or_else(|| anyhow!("failed to get user info after 5 attempts")))
            }
        };

        Ok(Some(info))
    });

    if let Some(info) = handle.await?? {
        let sign = info["sign"].as_str().ok_or_else(|| anyhow!("sign field not found or not a string"))?;
        let name = info["name"].as_str().ok_or_else(|| anyhow!("name field not found or not a string"))?;
        let level = info["level"].as_i64().ok_or_else(|| anyhow!("level field not found or not an integer"))? as i32;

        Ok(Some(BiliUserProfile {
            mid,
            name: name.to_string(),
            bio: sign.to_string(),
            level
        }))
    } else {
        Ok(None)
    }
}

#[tokio::test]
pub async fn test_get_user_profile() {
    let r = get_user_profile(2).await.unwrap();
    assert_eq!(r.is_some(), true);

    let r = r.unwrap();
    assert_eq!(r.mid, 2);
    assert_eq!(r.name, "碧诗");
    assert_eq!(r.bio, "https://kami.im 直男过气网红 #  We Are Star Dust");
    assert_eq!(r.level, 6);

    let r = get_user_profile(9223372036854775807).await.unwrap();
    assert_eq!(r.is_none(), true);
}