use hachimi_world_server::util;
use hachimi_world_server::util::bilibili::{BiliUserProfile, MockBilibiliClient};
use std::sync::{LazyLock, Mutex};
use tracing::info;

// This is used to provide a mock Bilibili client for testing. The mock client will return a fixed user profile for a specific mid, and return None for other mids. The user profile can be modified by calling set_test_user_bio function.
static BILIBILI_TEST_USER_PROFILE: LazyLock<Mutex<BiliUserProfile>> = LazyLock::new(|| Mutex::new(BiliUserProfile {
    mid: 123456,
    name: "测试用户".to_string(),
    bio: "这是一个测试用户".to_string(),
    level: 6,
}));

pub fn get_mock_bili_client() -> impl util::bilibili::BilibiliClient {
    let mut mock = MockBilibiliClient::default();
    mock.expect_user_info().withf(|mid| {
        info!("Mock get bilibili user info for mid {}", mid);
        true
    }).returning(|mid| {
        let mid = mid;
        Box::pin(async move {
            if mid == 123456 {
                Ok(Some(BILIBILI_TEST_USER_PROFILE.lock().unwrap().clone()))
            } else {
                Ok(None)
            }
        })
    });
    mock
}

pub fn set_mock_test_user_bio(bio: String) {
    let mut profile = BILIBILI_TEST_USER_PROFILE.lock().unwrap();
    profile.bio = bio;
}