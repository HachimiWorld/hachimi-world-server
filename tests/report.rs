mod common;

use crate::common::auth::{with_new_random_test_user, with_test_contributor_user, TestUser};
use crate::common::publish::{create_approved_song, publish_template};
use crate::common::{CommonParse, TestEnvironment};
use common::with_test_environment;
use hachimi_world_server::web::routes::committee::{MeResp, MemberReq, MembersResp};
use hachimi_world_server::web::routes::playlist::{AddSongReq, CreatePlaylistReq, CreatePlaylistResp, DetailReq as PlaylistDetailReq, DetailResp as PlaylistDetailResp, UpdatePlaylistReq};
use hachimi_world_server::web::routes::publish::review::ApproveReviewReq;
use hachimi_world_server::web::routes::publish::{ModifyReq, ModifyResp};
use hachimi_world_server::web::routes::report::{CaseReq, CaseResp, HiddenReasonReq, HiddenReasonResp, QueueReq, QueueResp, ResolveReq, ResolveResp, SubmitReq, SubmitResp};
use hachimi_world_server::web::routes::song::{DetailByIdReq, DetailResp as SongDetail, LikeReq, MyLikesReq, MyLikesResp};

fn login(env: &mut TestEnvironment, user: &TestUser) {
    env.api.set_token(user.token.access_token.clone());
}

fn report_req(target_type: &str, target_id: i64, reason: &str, detail: Option<&str>) -> SubmitReq {
    SubmitReq { target_type: target_type.into(), target_id, reason: reason.into(), detail: detail.map(Into::into) }
}

async fn submit(env: &TestEnvironment, req: &SubmitReq) -> Result<SubmitResp, String> {
    env.api.post("/report/submit", req).await.parse_resp().await.map_err(|e| e.code)
}

async fn queue(env: &TestEnvironment, status: Option<&str>) -> Result<QueueResp, String> {
    let req = QueueReq { status: status.map(Into::into), before_time: None, before_id: None, limit: None };
    env.api.get_query("/report/queue", &req).await.parse_resp().await.map_err(|e| e.code)
}

async fn case(env: &TestEnvironment, target_type: &str, target_id: i64) -> CaseResp {
    env.api.get_query("/report/case", &CaseReq { target_type: target_type.into(), target_id }).await.parse_resp().await.unwrap()
}

async fn resolve(env: &TestEnvironment, target_type: &str, target_id: i64, verdict: &str, ignore_reports: bool, up_to_report_id: i64) -> Result<ResolveResp, String> {
    decide(env, target_type, target_id, verdict, &[], None, ignore_reports, up_to_report_id).await
}

async fn decide(
    env: &TestEnvironment,
    target_type: &str,
    target_id: i64,
    verdict: &str,
    content_actions: &[&str],
    author_reason: Option<&str>,
    ignore_reports: bool,
    up_to_report_id: i64,
) -> Result<ResolveResp, String> {
    let req = ResolveReq {
        target_type: target_type.into(),
        target_id,
        verdict: verdict.into(),
        note: Some("checked".into()),
        content_actions: content_actions.iter().map(|x| x.to_string()).collect(),
        author_reason: author_reason.map(Into::into),
        ignore_reports,
        up_to_report_id,
    };
    env.api.post("/report/resolve", &req).await.parse_resp().await.map_err(|e| e.code)
}

/// (type, body) of a user's governance notifications, oldest first.
async fn governance_notifications(env: &TestEnvironment, uid: i64) -> Vec<(String, String)> {
    sqlx::query_as("SELECT notification_type, body FROM notifications WHERE recipient_uid = $1 AND notification_type LIKE 'governance.%' ORDER BY create_time, id")
        .bind(uid).fetch_all(&env.pool).await.unwrap()
}

async fn report_notifications(env: &TestEnvironment, uid: i64) -> Vec<String> {
    sqlx::query_scalar("SELECT body FROM notifications WHERE recipient_uid = $1 AND notification_type = 'governance.report_resolved' ORDER BY create_time")
        .bind(uid).fetch_all(&env.pool).await.unwrap()
}

/// Redis locks are released asynchronously; wait before contending for the same case again.
async fn wait_lock() {
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}

#[tokio::test]
async fn test_submit_validation() {
    with_test_environment(|mut env| async move {
        let contributor = with_test_contributor_user(&mut env).await;
        let owner = with_new_random_test_user(&mut env).await;
        let song = create_approved_song(&mut env, &owner.token.access_token, &contributor.token.access_token, "Reported").await;
        let reporter = with_new_random_test_user(&mut env).await;

        login(&mut env, &reporter);
        assert_eq!(submit(&env, &report_req("comment", song.id, "spam", None)).await.unwrap_err(), "invalid_target_type");
        assert_eq!(submit(&env, &report_req("song", -1, "spam", None)).await.unwrap_err(), "target_not_found");
        assert_eq!(submit(&env, &report_req("song", song.id, "boring", None)).await.unwrap_err(), "invalid_reason");
        assert_eq!(submit(&env, &report_req("song", song.id, "other", Some("  "))).await.unwrap_err(), "detail_required");
        assert_eq!(submit(&env, &report_req("user", reporter.uid, "spam", None)).await.unwrap_err(), "cannot_report_self");

        let first = submit(&env, &report_req("song", song.id, "spam", None)).await.unwrap();
        assert!(!first.already_reviewed);
        wait_lock().await;
        assert_eq!(submit(&env, &report_req("song", song.id, "abuse", None)).await.unwrap_err(), "already_reported");

        login(&mut env, &owner);
        assert_eq!(submit(&env, &report_req("song", song.id, "spam", None)).await.unwrap_err(), "cannot_report_self");

        // A private playlist can't be seen, so it can't be reported
        let playlist: CreatePlaylistResp = env.api.post("/playlist/create", &CreatePlaylistReq {
            name: "Private".into(), description: None, is_public: false,
        }).await.parse_resp().await.unwrap();
        login(&mut env, &reporter);
        assert_eq!(submit(&env, &report_req("playlist", playlist.id, "spam", None)).await.unwrap_err(), "target_not_found");
    }).await
}

#[tokio::test]
async fn test_reports_merge_into_one_case() {
    with_test_environment(|mut env| async move {
        let contributor = with_test_contributor_user(&mut env).await;
        let owner = with_new_random_test_user(&mut env).await;
        let song = create_approved_song(&mut env, &owner.token.access_token, &contributor.token.access_token, "Reported").await;
        let a = with_new_random_test_user(&mut env).await;
        let b = with_new_random_test_user(&mut env).await;
        let c = with_new_random_test_user(&mut env).await;

        for (user, reason) in [(&a, "spam"), (&b, "spam"), (&c, "nsfw")] {
            login(&mut env, user);
            submit(&env, &report_req("song", song.id, reason, None)).await.unwrap();
            wait_lock().await;
        }

        login(&mut env, &contributor);
        let q = queue(&env, None).await.unwrap();
        assert_eq!(q.items.len(), 1);
        let item = &q.items[0];
        assert_eq!((item.target_type.as_str(), item.target_id, item.pending_count), ("song", song.id, 3));
        assert!(item.target.as_ref().unwrap().title.starts_with("Reported"));
        assert_eq!(item.target.as_ref().unwrap().owner.as_ref().unwrap().uid, owner.uid);
        let reasons: Vec<(&str, i64)> = item.reasons.iter().map(|x| (x.reason.as_str(), x.count)).collect();
        assert_eq!(reasons, vec![("spam", 2), ("nsfw", 1)]);

        let detail = case(&env, "song", song.id).await;
        let reporters: Vec<i64> = detail.pending_reports.iter().map(|x| x.reporter.as_ref().unwrap().uid).collect();
        assert_eq!(reporters, vec![a.uid, b.uid, c.uid]);
        assert_eq!(detail.verdicts, vec!["agree", "disagree", "ignore"]);
        let actions: Vec<(&str, &str)> = detail.content_actions.iter().map(|x| (x.action.as_str(), x.verdict.as_str())).collect();
        assert_eq!(actions, vec![("hide", "agree")]);
    }).await
}

#[tokio::test]
async fn test_resolve_and_reopen() {
    with_test_environment(|mut env| async move {
        let contributor = with_test_contributor_user(&mut env).await;
        let target = with_new_random_test_user(&mut env).await;
        let a = with_new_random_test_user(&mut env).await;
        let b = with_new_random_test_user(&mut env).await;
        let c = with_new_random_test_user(&mut env).await;

        login(&mut env, &a);
        let first = submit(&env, &report_req("user", target.uid, "abuse", None)).await.unwrap();
        wait_lock().await;

        // Upholding reports must do something
        login(&mut env, &contributor);
        assert_eq!(resolve(&env, "user", target.uid, "agree", false, first.report_id).await.unwrap_err(), "content_action_required");
        wait_lock().await;

        // A report arriving after the operator looked stays pending
        login(&mut env, &b);
        let second = submit(&env, &report_req("user", target.uid, "spam", None)).await.unwrap();
        wait_lock().await;
        login(&mut env, &contributor);
        let r = resolve(&env, "user", target.uid, "disagree", false, first.report_id).await.unwrap();
        assert_eq!((r.status.as_str(), r.pending_count), ("pending", 1));
        assert_eq!(report_notifications(&env, a.uid).await.len(), 1);
        assert!(report_notifications(&env, a.uid).await[0].contains("未发现违规"));
        assert!(report_notifications(&env, b.uid).await.is_empty());
        wait_lock().await;

        let r = resolve(&env, "user", target.uid, "ignore", false, second.report_id).await.unwrap();
        assert_eq!((r.status.as_str(), r.pending_count), ("resolved", 0));
        assert!(report_notifications(&env, b.uid).await[0].contains("未采取措施"));
        assert!(queue(&env, None).await.unwrap().items.is_empty());
        assert_eq!(queue(&env, Some("resolved")).await.unwrap().items.len(), 1);
        wait_lock().await;

        // Reported again: the same case reopens, and the reporter can report again
        login(&mut env, &a);
        let third = submit(&env, &report_req("user", target.uid, "abuse", None)).await.unwrap();
        assert!(!third.already_reviewed);
        wait_lock().await;
        login(&mut env, &contributor);
        let q = queue(&env, None).await.unwrap();
        assert_eq!((q.items.len(), q.items[0].pending_count), (1, 1));
        let detail = case(&env, "user", target.uid).await;
        assert_eq!(detail.case.case_id, q.items[0].case_id);
        assert_eq!(detail.actions.len(), 2);
        assert_eq!(detail.actions[0].verdict, "ignore");

        // Ignore further reports: they are recorded without reopening the case
        resolve(&env, "user", target.uid, "disagree", true, third.report_id).await.unwrap();
        wait_lock().await;
        login(&mut env, &c);
        let fourth = submit(&env, &report_req("user", target.uid, "abuse", None)).await.unwrap();
        assert!(fourth.already_reviewed);
        login(&mut env, &contributor);
        assert!(queue(&env, None).await.unwrap().items.is_empty());
        assert!(report_notifications(&env, c.uid).await.is_empty());
        let action_id: Option<i64> = sqlx::query_scalar("SELECT action_id FROM reports WHERE id = $1")
            .bind(fourth.report_id).fetch_one(&env.pool).await.unwrap();
        assert!(action_id.is_some());
        wait_lock().await;

        // The case can take reports again, and then has nothing left to decide on
        assert_eq!(case(&env, "user", target.uid).await.verdicts, vec!["ignore"]);
        resolve(&env, "user", target.uid, "ignore", false, fourth.report_id).await.unwrap();
        wait_lock().await;
        assert!(case(&env, "user", target.uid).await.verdicts.is_empty());
        assert_eq!(resolve(&env, "user", target.uid, "ignore", false, fourth.report_id).await.unwrap_err(), "invalid_verdict");
    }).await
}

#[tokio::test]
async fn test_committee() {
    with_test_environment(|mut env| async move {
        let contributor = with_test_contributor_user(&mut env).await;
        let member = with_new_random_test_user(&mut env).await;
        let user = with_new_random_test_user(&mut env).await;

        // Others can't read the queue or manage members
        login(&mut env, &member);
        let me: MeResp = env.api.get("/committee/me").await.parse_resp().await.unwrap();
        assert!(!me.can_view && !me.can_resolve);
        assert_eq!(queue(&env, None).await.unwrap_err(), "permission_denied");
        let err = env.api.post("/committee/appoint", &MemberReq { uid: member.uid }).await.parse_resp::<()>().await.unwrap_err();
        assert_eq!(err.code, "permission_denied");

        login(&mut env, &contributor);
        let me: MeResp = env.api.get("/committee/me").await.parse_resp().await.unwrap();
        assert!(me.can_view && me.can_resolve);
        env.api.post("/committee/appoint", &MemberReq { uid: member.uid }).await.parse_resp::<()>().await.unwrap();
        wait_lock().await;
        let err = env.api.post("/committee/appoint", &MemberReq { uid: member.uid }).await.parse_resp::<()>().await.unwrap_err();
        assert_eq!(err.code, "already_member");
        let err = env.api.post("/committee/appoint", &MemberReq { uid: -1 }).await.parse_resp::<()>().await.unwrap_err();
        assert_eq!(err.code, "user_not_found");

        // Members read the queue but can't decide
        login(&mut env, &user);
        let report = submit(&env, &report_req("user", contributor.uid, "spam", None)).await.unwrap();
        wait_lock().await;
        login(&mut env, &member);
        let me: MeResp = env.api.get("/committee/me").await.parse_resp().await.unwrap();
        assert!(me.can_view && !me.can_resolve);
        assert_eq!(queue(&env, None).await.unwrap().items.len(), 1);
        assert_eq!(resolve(&env, "user", contributor.uid, "ignore", false, report.report_id).await.unwrap_err(), "permission_denied");
        let list: MembersResp = env.api.get("/committee/members").await.parse_resp().await.unwrap();
        assert_eq!(list.members.len(), 1);
        assert_eq!(list.members[0].user.uid, member.uid);
        assert_eq!(list.members[0].appointed_by.as_ref().unwrap().uid, contributor.uid);

        login(&mut env, &contributor);
        env.api.post("/committee/revoke", &MemberReq { uid: member.uid }).await.parse_resp::<()>().await.unwrap();
        let err = env.api.post("/committee/revoke", &MemberReq { uid: member.uid }).await.parse_resp::<()>().await.unwrap_err();
        assert_eq!(err.code, "not_member");
        login(&mut env, &member);
        assert_eq!(queue(&env, None).await.unwrap_err(), "permission_denied");
    }).await
}

async fn song_detail(env: &TestEnvironment, song_id: i64) -> Result<SongDetail, String> {
    env.api.get_query("/song/detail_by_id", &DetailByIdReq { id: song_id }).await.parse_resp().await.map_err(|e| e.code)
}

#[tokio::test]
async fn test_hide_and_restore_song() {
    with_test_environment(|mut env| async move {
        let contributor = with_test_contributor_user(&mut env).await;
        let owner = with_new_random_test_user(&mut env).await;
        let song = create_approved_song(&mut env, &owner.token.access_token, &contributor.token.access_token, "Hidden").await;
        let fan = with_new_random_test_user(&mut env).await;
        let reporter = with_new_random_test_user(&mut env).await;

        // A fan likes the song and keeps it in a public playlist
        login(&mut env, &fan);
        env.api.post("/song/likes/like", &LikeReq { song_id: song.id, playback_position_secs: None }).await
            .parse_resp::<serde_json::Value>().await.unwrap();
        let playlist: CreatePlaylistResp = env.api.post("/playlist/create", &CreatePlaylistReq {
            name: "Fan".into(), description: None, is_public: true,
        }).await.parse_resp().await.unwrap();
        env.api.post("/playlist/add_song", &AddSongReq { playlist_id: playlist.id, song_id: song.id }).await
            .parse_resp::<serde_json::Value>().await.unwrap();

        login(&mut env, &reporter);
        let report = submit(&env, &report_req("song", song.id, "nsfw", None)).await.unwrap();
        wait_lock().await;

        login(&mut env, &contributor);
        assert_eq!(decide(&env, "song", song.id, "agree", &[], None, false, report.report_id).await.unwrap_err(), "content_action_required");
        wait_lock().await;
        assert_eq!(decide(&env, "song", song.id, "agree", &["hide"], None, false, report.report_id).await.unwrap_err(), "author_reason_required");
        wait_lock().await;
        assert_eq!(decide(&env, "song", song.id, "disagree", &["hide"], None, false, report.report_id).await.unwrap_err(), "invalid_content_action");
        wait_lock().await;
        assert_eq!(decide(&env, "song", song.id, "agree", &["restore"], Some("x"), false, report.report_id).await.unwrap_err(), "invalid_content_action");
        wait_lock().await;
        decide(&env, "song", song.id, "agree", &["hide"], Some("封面不适合公开展示"), false, report.report_id).await.unwrap();
        wait_lock().await;

        // Only the owner can still see it
        env.api.clear_token();
        assert_eq!(song_detail(&env, song.id).await.unwrap_err(), "not_found");
        login(&mut env, &fan);
        assert_eq!(song_detail(&env, song.id).await.unwrap_err(), "not_found");
        login(&mut env, &owner);
        assert!(song_detail(&env, song.id).await.unwrap().is_hidden);
        let notice: HiddenReasonResp = env.api.get_query("/report/hidden_reason", &HiddenReasonReq { target_type: "song".into(), target_id: song.id })
            .await.parse_resp().await.unwrap();
        assert!(notice.hidden);
        assert_eq!(notice.reason.as_deref(), Some("封面不适合公开展示"));

        // It stays in place as unavailable in the fan's playlist and likes
        login(&mut env, &fan);
        let detail: PlaylistDetailResp = env.api.get_query("/playlist/detail", &PlaylistDetailReq { id: playlist.id }).await.parse_resp().await.unwrap();
        assert!(detail.songs.is_empty());
        assert_eq!(detail.unavailable_songs.iter().map(|x| x.song_id).collect::<Vec<_>>(), vec![song.id]);
        assert_eq!(detail.playlist_info.songs_count, 1);
        let likes: MyLikesResp = env.api.get_query("/song/likes/page_my_likes", &MyLikesReq { page_index: 0, page_size: 20 }).await.parse_resp().await.unwrap();
        assert!(likes.data.is_empty());
        assert_eq!(likes.unavailable.iter().map(|x| x.song_id).collect::<Vec<_>>(), vec![song.id]);
        let notice: HiddenReasonResp = env.api.get_query("/report/hidden_reason", &HiddenReasonReq { target_type: "song".into(), target_id: song.id })
            .await.parse_resp().await.unwrap();
        assert!(!notice.hidden && notice.reason.is_none());

        // A hidden song can't be reported or added to playlists
        assert_eq!(submit(&env, &report_req("song", song.id, "spam", None)).await.unwrap_err(), "target_not_found");
        let err = env.api.post("/playlist/add_song", &AddSongReq { playlist_id: playlist.id, song_id: song.id }).await
            .parse_resp::<serde_json::Value>().await.unwrap_err();
        assert_eq!(err.code, "song_not_found");

        let reporter_notes = governance_notifications(&env, reporter.uid).await;
        assert!(reporter_notes[0].1.contains("已采取相应措施"));
        let owner_notes = governance_notifications(&env, owner.uid).await;
        assert_eq!(owner_notes[0].0, "governance.content_hidden");
        assert!(owner_notes[0].1.contains("原因：封面不适合公开展示"));

        // Without pending reports, it can only be restored
        login(&mut env, &contributor);
        let detail = case(&env, "song", song.id).await;
        assert_eq!(detail.verdicts, vec!["disagree"]);
        let actions: Vec<(&str, &str)> = detail.content_actions.iter().map(|x| (x.action.as_str(), x.verdict.as_str())).collect();
        assert_eq!(actions, vec![("restore", "disagree")]);
        decide(&env, "song", song.id, "disagree", &["restore"], None, false, 0).await.unwrap();

        env.api.clear_token();
        assert!(!song_detail(&env, song.id).await.unwrap().is_hidden);
        login(&mut env, &fan);
        let detail: PlaylistDetailResp = env.api.get_query("/playlist/detail", &PlaylistDetailReq { id: playlist.id }).await.parse_resp().await.unwrap();
        assert_eq!((detail.songs.len(), detail.unavailable_songs.len()), (1, 0));
        assert_eq!(governance_notifications(&env, owner.uid).await[1].0, "governance.content_restored");
    }).await
}

#[tokio::test]
async fn test_modification_lifts_hide() {
    with_test_environment(|mut env| async move {
        let contributor = with_test_contributor_user(&mut env).await;
        let owner = with_new_random_test_user(&mut env).await;
        let song = create_approved_song(&mut env, &owner.token.access_token, &contributor.token.access_token, "Fixable").await;
        let reporter = with_new_random_test_user(&mut env).await;
        login(&mut env, &reporter);
        let report = submit(&env, &report_req("song", song.id, "copyright", None)).await.unwrap();
        wait_lock().await;
        login(&mut env, &contributor);
        decide(&env, "song", song.id, "agree", &["hide"], Some("歌词侵权"), false, report.report_id).await.unwrap();

        login(&mut env, &owner);
        let template = publish_template(&env).await;
        let modified: ModifyResp = env.api.post("/publish/modify", &ModifyReq {
            song_id: song.id,
            song_temp_id: None,
            cover_temp_id: None,
            title: template.title.clone(),
            subtitle: "Fixed".into(),
            description: template.description.clone(),
            lyrics: template.lyrics.clone(),
            tag_ids: template.tag_ids.clone(),
            creation_info: template.creation_info.clone(),
            production_crew: template.production_crew.clone(),
            external_links: template.external_links.clone(),
            explicit: false,
            comment: None,
        }).await.parse_resp().await.unwrap();
        login(&mut env, &contributor);
        env.api.post("/publish/review/approve", &ApproveReviewReq { review_id: modified.review_id, comment: None }).await
            .parse_resp::<serde_json::Value>().await.unwrap();

        env.api.clear_token();
        assert!(!song_detail(&env, song.id).await.unwrap().is_hidden);
        let body: String = sqlx::query_scalar("SELECT body FROM notifications WHERE recipient_uid = $1 AND notification_type = 'publish.modify_approved'")
            .bind(owner.uid).fetch_one(&env.pool).await.unwrap();
        assert!(body.ends_with("作品已恢复显示。"));
    }).await
}

#[tokio::test]
async fn test_hide_playlist() {
    with_test_environment(|mut env| async move {
        let contributor = with_test_contributor_user(&mut env).await;
        let owner = with_new_random_test_user(&mut env).await;
        let reporter = with_new_random_test_user(&mut env).await;
        login(&mut env, &owner);
        let playlist: CreatePlaylistResp = env.api.post("/playlist/create", &CreatePlaylistReq {
            name: "Spam".into(), description: None, is_public: true,
        }).await.parse_resp().await.unwrap();

        login(&mut env, &reporter);
        let report = submit(&env, &report_req("playlist", playlist.id, "spam", None)).await.unwrap();
        wait_lock().await;
        login(&mut env, &contributor);
        decide(&env, "playlist", playlist.id, "agree", &["hide"], Some("引流"), false, report.report_id).await.unwrap();

        login(&mut env, &reporter);
        let err = env.api.get_query("/playlist/detail", &PlaylistDetailReq { id: playlist.id }).await
            .parse_resp::<PlaylistDetailResp>().await.unwrap_err();
        assert_eq!(err.code, "not_found");
        login(&mut env, &owner);
        let detail: PlaylistDetailResp = env.api.get_query("/playlist/detail", &PlaylistDetailReq { id: playlist.id }).await.parse_resp().await.unwrap();
        assert!(detail.playlist_info.is_hidden);
        // Editing it doesn't bring it back
        env.api.post("/playlist/update", &UpdatePlaylistReq { id: playlist.id, name: "Clean".into(), description: None, is_public: true }).await
            .parse_resp::<serde_json::Value>().await.unwrap();
        login(&mut env, &reporter);
        assert!(env.api.get_query("/playlist/detail", &PlaylistDetailReq { id: playlist.id }).await
            .parse_resp::<PlaylistDetailResp>().await.is_err());
    }).await
}

#[tokio::test]
async fn test_reset_profile() {
    with_test_environment(|mut env| async move {
        let contributor = with_test_contributor_user(&mut env).await;
        let target = with_new_random_test_user(&mut env).await;
        let reporter = with_new_random_test_user(&mut env).await;
        sqlx::query("UPDATE users SET bio = 'bad bio', avatar_url = 'https://example.com/a.png' WHERE id = $1")
            .bind(target.uid).execute(&env.pool).await.unwrap();

        login(&mut env, &reporter);
        let report = submit(&env, &report_req("user", target.uid, "abuse", None)).await.unwrap();
        wait_lock().await;
        login(&mut env, &contributor);
        let detail = case(&env, "user", target.uid).await;
        let actions: Vec<&str> = detail.content_actions.iter().map(|x| x.action.as_str()).collect();
        assert_eq!(actions, vec!["reset_avatar", "reset_bio", "reset_username"]);
        decide(&env, "user", target.uid, "agree", &["reset_bio", "reset_username"], Some("简介含辱骂内容"), false, report.report_id).await.unwrap();

        let (username, bio, avatar): (String, Option<String>, Option<String>) =
            sqlx::query_as("SELECT username, bio, avatar_url FROM users WHERE id = $1").bind(target.uid).fetch_one(&env.pool).await.unwrap();
        assert!(username.starts_with("神人") && username != target.name);
        assert_eq!(bio, None);
        assert_eq!(avatar.as_deref(), Some("https://example.com/a.png"));
        let notes = governance_notifications(&env, target.uid).await;
        assert_eq!(notes[0].0, "governance.profile_reset");
        assert!(notes[0].1.starts_with("你的简介、昵称已被重置。"));
    }).await
}
