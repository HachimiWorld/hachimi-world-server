use anyhow::{anyhow, bail};

pub struct BiliUserProfile {
    pub mid: i64,
    pub name: String,
    pub bio: String,
    pub level: i32
}

pub async fn get_user_profile(mid: i64) -> anyhow::Result<BiliUserProfile> {
    let handle = tokio::task::spawn(async move {
        let mut cli = bilibili_api_rs::Client::new();
        let mut init_cnt = 0;
        let info = loop {
            if let Ok(v) = cli.user(mid).info().await {
                break v;
            }
            init_cnt += 1;
            if init_cnt > 5 {
                bail!("init retry too many: {init_cnt}");
            }
        };
        Ok(info)
    });
    let info = handle.await??;
    let sign = info["sign"].as_str().ok_or_else(|| anyhow!("sign field not found or not a string"))?;
    let name = info["name"].as_str().ok_or_else(|| anyhow!("name field not found or not a string"))?;
    let level = info["level"].as_i64().ok_or_else(|| anyhow!("level field not found or not an integer"))? as i32;

    Ok(BiliUserProfile {
        mid,
        name: name.to_string(),
        bio: sign.to_string(),
        level
    })
}

#[tokio::test]
pub async fn test_get_user_profile() {
    let r = get_user_profile(2).await.unwrap();
    assert_eq!(r.mid, 2);
    assert_eq!(r.name, "碧诗");
    assert_eq!(r.bio, "https://kami.im 直男过气网红 #  We Are Star Dust");
    assert_eq!(r.level, 6);
}