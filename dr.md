# MIZ.csv に基づく最小 MVP 実装計画

## エグゼクティブサマリー

アップロード済みの **MIZ.csv は提供済み**であり、直接解析できました。CSV の構造は解析時点で確定し、**`TYPE=task` を TODO 項目、`TYPE=note` を直前 task に付随する authoritative comments** として扱うのが妥当です。解析結果は、**15 列、task 92 行、note 75 行、section 3 行、meta 1 行、空行 4 行**でした。明示的な `TODO` 文字列はわずか 2 件ですが、このファイルでは **`TYPE=task` 自体が TODO の実体**であり、加えて `@mvp` と `@phase-*` が優先度と実装段階の主要マーカーとして使われています。

MVP として最低限 handoff すべき範囲は、CSV 上の `@mvp` タスクと、それを成立させる priority 1 の foundation タスクを重ね合わせると、次の五点に収束します。  
**登録・ログイン・セッション管理**、**プロフィール編集・ハンドル・公開設定**、**短文投稿・Reply**、**Follow・承認・ホームタイムライン**、**共通 API 契約・エラー・CI/配備基盤**です。これより先の **通知、検索、メディア投稿、チャット、ActivityPub 連合、運営管理、Block/Mute、削除復元の完全フロー**は、CSV 上でも post-MVP または hardening 側に寄っており、最小のユーザー価値を出す MVP からは外すべきです。

技術スタックは CSV から **Rust + Axum / SvelteKit + TypeScript / PostgreSQL / Docker Compose / OpenAPI** と読めるため、本レポートでは **Rust（Axum）+ SvelteKit** を Codex 向けの一次テンプレートとして採用します。Axum は Router と extractor ベースの HTTP 処理を提供し、SvelteKit は server routes / form actions / load によるフロント統合を提供します。PostgreSQL は `CREATE TABLE` / `CREATE INDEX` を前提にした素直な RDB 設計が可能で、Docker Compose は複数サービスの定義と起動、GitHub Actions は `.github/workflows` 配下の YAML による CI を標準提供します。OpenAPI は HTTP API の言語非依存な契約記述であり、エラー形式は RFC 9457 の Problem Details に揃えるのが最小で堅牢です。citeturn0search0turn0search1turn0search2turn0search3turn1search0turn1search5turn2search0

## CSV 解析サマリー

### 解析済みサマリー

| 項目 | 解析結果 |
|---|---|
| CSV 提供有無 | 提供済み |
| 文字コード | UTF-8 with BOM |
| 検出列 | `TYPE`, `CONTENT`, `DESCRIPTION`, `IS_COLLAPSED`, `PRIORITY`, `INDENT`, `AUTHOR`, `RESPONSIBLE`, `DATE`, `DATE_LANG`, `TIMEZONE`, `DURATION`, `DURATION_UNIT`, `DEADLINE`, `DEADLINE_LANG` |
| 総行数 | 175 |
| task 行 | 92 |
| note 行 | 75 |
| section 行 | 3 |
| meta 行 | 1 |
| 空行 | 4 |
| task に note が付属 | 74 task |
| note なし task | 18 task |
| 明示的 `TODO`/`todo` 文字列 | 2 件 |
| 主要マーカー | `@mvp` 7 件、`@phase-00-foundation` 21 件、`@phase-10-mvp-backend-identity` 12 件、`@phase-20-mvp-backend-content` 3 件、`@phase-30-mvp-backend-social` 4 件 |
| スタック検出 | Rust / Axum / SvelteKit / TypeScript / PostgreSQL / Docker Compose / OpenAPI |

### task と note の対応ルール

今回の CSV では、`TYPE=note` は常に**最も近い直前の `TYPE=task`** に付属する comment として扱うのが構造上もっとも自然でした。これを機械的に適用すると、`@mvp` task 7 件のうち note 付きは 4 件、note なしは 3 件です。note を持たない task については、同じテーマの foundation task 群にある schema / endpoint / security コメントで補完する必要があります。

### 抽出した主要 TODO 群

| 区分 | 抽出対象 |
|---|---|
| 直接の MVP task | Account Create, Login Methods, 短文投稿, Reply, Profile Edit, Follow, フォローしたユーザーの投稿をタイムラインに流しやすくする |
| MVP を成立させる foundation task | Stack / Architecture, Public Object IDs, Handle / Federation Identity, User / Session Schema, Post / Reply Schema, Follow / Block / Mute State Machine, Registration / Consent / Challenge Schema, API Contract, API Conventions / Error Model, Account / Session Endpoints, Post / Reply / Timeline Endpoints, Relationship Endpoints, Cursor / Concurrency / Idempotency, Registration / Consent Endpoints, Authentication / Authorization / Security, Development / Release Foundation |
| MVP から外す task 群 | Notification, Search, 画像投稿, Chat, Core Federation, Moderation Administration, Custom Emoji Federation, post-MVP frontend, release hardening の一部 |

CSV から読めるスタック指定は、Axum の Router / extractor モデル、SvelteKit の routing / load / form actions、PostgreSQL のテーブル・インデックス定義、Docker Compose の multi-container 構成、GitHub Actions の YAML ワークフローとよく整合します。したがって、実装 plan もそのままこの方向で固定するのが最小です。citeturn0search0turn0search1turn0search2turn0search3turn1search0

## MVP 選定と優先順位

### MVP 採用タスクと優先度

| MVP 項目 | CSV 上の直接 task | 主な補助 task | 優先度 | 工数 | 採用理由 |
|---|---|---|---|---|---|
| 登録・ログイン・セッション | Account Create, Login Methods | Registration Policy, Session Management, Registration / Consent Endpoints, Authentication / Authorization / Security | P0 | Large | これがないとユーザー存在と継続ログインが成立しない |
| プロフィール・ハンドル・公開設定 | Profile Edit | Account Privacy, Handle / Federation Identity, User / Session Schema, Account / Session Endpoints | P0 | Medium | 投稿可視性と Follow 承認に直結する |
| 短文投稿・Reply | 短文投稿, Reply | Post / Reply Schema, Post / Reply / Timeline Endpoints, Cursor / Concurrency / Idempotency | P0 | Medium | もっとも小さいコア体験 |
| Follow・承認・ホームタイムライン | Follow, フォローしたユーザーの投稿をタイムラインに流しやすくする | Follow / Block / Mute State Machine, Relationship Endpoints, Account Privacy | P0 | Medium〜Large | ソーシャル体験の最小単位 |
| 共通 API・エラー・配備基盤 | なし | API Contract, API Conventions / Error Model, Development / Release Foundation, Stack / Architecture | P0 | Medium | Codex handoff と実装収束に必須 |

### MVP から外す項目

| 除外項目 | 理由 |
|---|---|
| 画像投稿・添付メディア | `短文投稿` note に添付の痕跡はあるが、専用の `画像投稿` task が post-MVP 側にあり、MVP を本文のみで切る方が最小 |
| Search / Notification | post-MVP 明示。コア体験成立に不要 |
| Block / Mute | 重要だが hardening フェーズ。初回出荷の価値対工数が低い |
| Moderation / 通報 | 内部運用機能で出荷最小条件ではない |
| Core Federation / ActivityPub / Custom Emoji Federation | 明確に post-MVP |
| Realtime transport | foundation 側に SSE 契約の痕跡はあるが、専用 realtime task が post-MVP のため、初回は polling で十分 |
| 削除復元の完全状態機械 | foundation にはあるが、MVP の直接価値より先に出す必要はない |

この切り方は、**CSV の authoritative comments を維持しつつ、相互に競合する記述がある部分を最小実装へ収束させる**という方針です。アカウント作成は `POST /api/v1/registrations`、ログインは `POST /api/v1/auth/login` に限定し、メール認証の中間状態を持たないことで API 面積を最小化します。また、SSE は後から `EventSource` ベースで追加しやすいため、初回はホームタイムラインの polling で十分です。`EventSource` はサーバーから `text/event-stream` でイベントを継続送信するブラウザ API です。citeturn2search3turn2search7

## MVP 項目別仕様

### MVP 項目比較表

| 項目 | 目標 | 主要入力 | 主要出力 | 主テーブル | API 数 | 工数 |
|---|---|---|---|---|---|---|
| 登録・ログイン・セッション | 登録・ログイン・Cookie セッション発行 | email, Google callback, birthDate, consents | user, session, registration state | `registration_attempts`, `auth_challenges`, `auth_identities`, `user_private_attributes`, `user_consents`, `sessions` | 11 | Large |
| プロフィール・ハンドル・公開設定 | ハンドル・displayName・bio・privacy 編集 | handle, displayName, bio, privacy | self/public profile | `users`, `handle_aliases` | 4 | Medium |
| 短文投稿・Reply | 本文投稿・返信・本人編集削除 | content, replyToPostId | post/reply | `posts`, `idempotency_keys` | 6 | Medium |
| Follow・承認・ホームタイムライン | follow request と accepted feed | targetUserId, cursor | relationship, timeline | `follow_relationships` | 8 | Medium〜Large |
| 共通 API・配備基盤 | 契約・エラー・CI を固定 | HTTP request 全般 | OpenAPI, Problem Details, build artifacts | 既存全体 | 3 | Medium |

### 項目別詳細

| 項目 | 具体仕様 |
|---|---|
| 登録・ログイン・セッション | **Goal**: MVPではメール認証を使用せず、username + password でaccountを作成・ログインし、opaque session cookieを発行する。 **Acceptance criteria**: usernameはcase-insensitiveで一意、passwordは12〜128 bytesでArgon2idによりsalt付きhashとして保存し、認証失敗はusernameの存在を露出しない。セッションはidle 7日 / absolute 30日、一覧とrevokeを提供する。 **Inputs/Outputs**: 登録入力は`username`, `password`, `displayName`, `birthDate`、ログイン入力は`username`, `password`。出力は`user`とsession cookie。 **Data model**: `password_credentials(user_id,password_hash,created_at,updated_at)`, `sessions(...)`。 **Endpoints**: `POST /api/v1/registrations`, `POST /api/v1/auth/login`, `GET /api/v1/sessions`, `DELETE /api/v1/sessions/{sessionId}`, `DELETE /api/v1/sessions/current`。 **Key logic**: account・credential・初回sessionを単一transactionで作成し、Origin検証とrate limitを適用する。 **Error cases**: `invalid_credentials`, `handle_conflict`, `csrf_failed`, `rate_limited`。 **Dependencies**: PostgreSQL, Argon2id, Cookie/CSRF middleware。 **Minimal tests**: salt付きhash、登録とログイン、重複username、誤password、セッションrevoke。Passkeyはpost-MVP。 |
| プロフィール・ハンドル・公開設定 | **Goal**: `handle`, `displayName`, `bio`, `privacy(public/private)` を編集し、公開範囲をサーバー側で判定する。 **Acceptance criteria**: handle は 3〜24 の `[a-z0-9_]`、case-insensitive 一意、旧 handle の再利用禁止、プロフィール更新は `If-Match` 必須、private account の投稿は非 follower へ非表示。 **Inputs/Outputs**: `handle`, `displayName`, `bio`, `privacy` → `user profile`。 **Data model**: `users(id,handle,display_name,bio,privacy,status,version,created_at,updated_at)`, `handle_aliases(old_handle,user_id,created_at)`。 **Endpoints**: `GET /api/v1/users/me`, `PATCH /api/v1/users/me`, `GET /api/v1/users/{userId}`, `GET /api/v1/handles/{handle}`。 **Key logic**: handle 変更時は正規化→予約語判定→一意性確認→ alias 留保。 privacy は閲覧判定共通関数へ寄せる。 **Error cases**: `invalid_handle`, `reserved_handle`, `handle_conflict`, `version_conflict`, `resource_not_found`。 **Dependencies**: 認証済み user, sessions, optimistic locking。 **Minimal tests**: handle 一意性、旧 handle lookup、private account 非表示、If-Match 不一致。 |
| 短文投稿・Reply | **Goal**: 1〜500 grapheme の本文投稿と reply を作成し、本人のみ編集削除できる。 **Acceptance criteria**: 空白のみ禁止、改行維持、1〜500 grapheme、UTF-8 8KB 以下、Reply は親より広い可視性を持てない、`POST` は `Idempotency-Key`、`PATCH`/`DELETE` は `If-Match` 必須。 **Inputs/Outputs**: `content`, `replyToPostId`, headers → `post/reply`。 **Data model**: `posts(id,author_id,reply_to_post_id,content,effective_visibility,state,version,created_at,updated_at,deleted_at)`、`idempotency_keys(...)`。 **Endpoints**: `POST /api/v1/posts`, `GET /api/v1/posts/{postId}`, `PATCH /api/v1/posts/{postId}`, `DELETE /api/v1/posts/{postId}`, `POST /api/v1/posts/{postId}/replies`, `GET /api/v1/posts/{postId}/replies`。 **Key logic**: grapheme cluster 単位の長さ判定、サーバー側 visibility 決定、Reply は `min(parent_visibility, author_privacy)`、削除は tombstone。 **Error cases**: `content_empty`, `content_too_long`, `parent_not_visible`, `idempotency_required`, `idempotency_conflict`, `version_conflict`。 **Dependencies**: users, privacy 判定, follow relationship, idempotency storage。 **Minimal tests**: emoji 境界、空白のみ拒否、同一 `Idempotency-Key` 再送、親非表示 reply 拒否。 |
| Follow・承認・ホームタイムライン | **Goal**: public account には即 accepted、private account には pending follow request を作成し、accepted relation と self の投稿だけで home timeline を出す。 **Acceptance criteria**: self follow 禁止、pending→accepted/rejected、unfollow/cancel/remove が冪等、home timeline は `created_at DESC, id DESC` keyset pagination、可視でない投稿を返さない。 **Inputs/Outputs**: `targetUserId`, `cursor`, `limit` → `relationship`, `timeline page`。 **Data model**: `follow_relationships(id,follower_id,followee_id,status,created_at,updated_at)`。 **Endpoints**: `PUT /api/v1/users/{targetUserId}/follow`, `DELETE /api/v1/users/{targetUserId}/follow`, `GET /api/v1/follow-requests`, `POST /api/v1/follow-requests/{relationshipId}/accept`, `POST /api/v1/follow-requests/{relationshipId}/reject`, `GET /api/v1/users/{userId}/followers`, `GET /api/v1/users/{userId}/following`, `GET /api/v1/timelines/home`。 **Key logic**: state machine 厳守、timeline は recommendation を入れず self + accepted follow only、cursor は署名付き keyset。 **Error cases**: `cannot_follow_self`, `invalid_state_transition`, `target_not_visible`, `invalid_cursor`, `cursor_expired`。 **Dependencies**: users, privacy, posts, cursor signer。 **Minimal tests**: public 即 accepted、private pending→accept、home timeline 順序安定、解除後除外。 |
| 共通 API・配備基盤 | **Goal**: `/api/v1`、OpenAPI 契約、Problem Details、Cookie/CSRF、Docker/CI を固定する。 **Acceptance criteria**: OpenAPI を契約の正本、JSON は camelCase、日時は RFC3339 UTC、Problem Details は `application/problem+json`、GitHub Actions で format/lint/test/build を実行。 **Inputs/Outputs**: 全 HTTP request → 一貫した success/error response。 **Data model**: `idempotency_keys(user_id,endpoint,idempotency_key,request_hash,response_status,response_body,expires_at)`。 **Endpoints**: `GET /healthz`, `GET /readyz`, `GET /openapi.json`。 **Key logic**: deny-by-default authz、Cookie セッション、CSRF token + Origin 検証、OAuth callback 検証、OpenAPI から型生成。 **Error cases**: `auth_required`, `forbidden`, `precondition_required`, `problem_validation_failed`。 **Dependencies**: Axum middleware, OpenAPI document, Docker, GitHub Actions。 **Minimal tests**: 401/403/404/409/412/428、Problem Details 形式、Cookie 属性、CSRF 拒否、OpenAPI 整合。 |

### OpenAPI 形式の簡約エンドポイント表

| Method | Path | Request JSON | Response JSON |
|---|---|---|---|
| POST | `/api/v1/registrations` | `{"username":"miz_user","password":"correct horse battery staple","displayName":"Miz User","birthDate":"2000-01-01"}` | `201 {"id":"u_01...","handle":"miz_user","displayName":"Miz User","privacy":"public","version":1}` |
| POST | `/api/v1/auth/login` | `{"username":"miz_user","password":"correct horse battery staple"}` | `200 {"id":"u_01...","handle":"miz_user","displayName":"Miz User","privacy":"public","version":1}` |
| GET | `/api/v1/users/me` | なし | `200 {"id":"u_01...","handle":"miz_user","displayName":"Miz User","bio":"","privacy":"public","version":3}` |
| PATCH | `/api/v1/users/me` | `{"displayName":"Miz v2","bio":"Hello","privacy":"private"}` | `200 {"id":"u_01...","displayName":"Miz v2","bio":"Hello","privacy":"private","version":4}` |
| POST | `/api/v1/posts` | `{"content":"こんにちは"}` | `201 {"id":"p_01...","authorId":"u_01...","replyToPostId":null,"content":"こんにちは","effectiveVisibility":"public","state":"published","version":1}` |
| POST | `/api/v1/posts/{postId}/replies` | `{"content":"返信です"}` | `201 {"id":"p_02...","replyToPostId":"p_01...","content":"返信です","effectiveVisibility":"followers","state":"published","version":1}` |
| PUT | `/api/v1/users/{targetUserId}/follow` | なし | `200 {"id":"fr_01...","followerId":"u_me","followeeId":"u_target","status":"accepted"}` |
| GET | `/api/v1/timelines/home?limit=30&cursor=...` | なし | `200 {"items":[...],"nextCursor":"eyJ..."}` |

OpenAPI は HTTP API の言語非依存な契約記述であり、Problem Details は HTTP API エラーの標準フォーマットです。Cookie ベース認証を採る場合は `Secure`・`HttpOnly`・`SameSite` を明示し、状態変更系は CSRF 対策を入れるのが最小で安全です。Google のサーバーサイド OAuth は web server application フローを前提にでき、PKCE は code interception 対策として標準化されています。citeturn1search5turn2search0turn4search1turn4search5turn3search1turn2search1turn2search2

## Codex 向けコードテンプレート

CSV が **Rust/Axum + SvelteKit** を明示しているため、ここではその二言語を一次テンプレートとして示します。FastAPI / Express のデフォルトテンプレートは、**スタックが未指定ではないため省略**します。Axum は `Router`、`State` extractor、JSON extractors を使った構成に向いており、SvelteKit は `+server.ts` と `RequestHandler` で最小の BFF / mock route を構成できます。citeturn0search0turn0search8turn0search9turn0search13

### Rust Axum テンプレート

```toml
# apps/api/Cargo.toml
[package]
name = "miz-api"
version = "0.1.0"
edition = "2021"

[dependencies]
axum = "0.8"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tower = "0.5"
http-body-util = "0.1"
unicode-segmentation = "1.12"
```

```rust
// apps/api/src/main.rs
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::{
    sync::{Arc, Mutex},
};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Default)]
struct InMemoryRepo {
    posts: Arc<Mutex<Vec<PostDto>>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreatePostRequest {
    content: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PostDto {
    id: String,
    author_id: String,
    reply_to_post_id: Option<String>,
    content: String,
    effective_visibility: String,
    state: String,
    version: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ApiProblem {
    r#type: String,
    title: String,
    status: u16,
    detail: String,
    code: String,
}

impl ApiProblem {
    fn new(status: StatusCode, code: &str, detail: &str) -> Self {
        Self {
            r#type: format!("https://api.miz.local/problems/{code}"),
            title: status.canonical_reason().unwrap_or("Error").to_string(),
            status: status.as_u16(),
            detail: detail.to_string(),
            code: code.to_string(),
        }
    }
}

impl IntoResponse for ApiProblem {
    fn into_response(self) -> Response {
        (
            StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            [("content-type", "application/problem+json")],
            Json(self),
        )
            .into_response()
    }
}

async fn create_post(
    State(repo): State<InMemoryRepo>,
    headers: HeaderMap,
    Json(body): Json<CreatePostRequest>,
) -> Result<(StatusCode, Json<PostDto>), ApiProblem> {
    if headers.get("idempotency-key").is_none() {
        return Err(ApiProblem::new(
            StatusCode::PRECONDITION_REQUIRED,
            "idempotency_required",
            "Idempotency-Key header is required",
        ));
    }

    let trimmed = body.content.trim();
    if trimmed.is_empty() {
        return Err(ApiProblem::new(
            StatusCode::BAD_REQUEST,
            "content_empty",
            "content must not be blank",
        ));
    }

    let grapheme_len = UnicodeSegmentation::graphemes(trimmed, true).count();
    if !(1..=500).contains(&grapheme_len) {
        return Err(ApiProblem::new(
            StatusCode::BAD_REQUEST,
            "content_too_long",
            "content must be between 1 and 500 graphemes",
        ));
    }

    if trimmed.as_bytes().len() > 8_000 {
        return Err(ApiProblem::new(
            StatusCode::BAD_REQUEST,
            "content_too_large",
            "content must be <= 8000 bytes in UTF-8",
        ));
    }

    let post = PostDto {
        id: "p_01TESTPOSTID000000000".to_string(),
        author_id: "u_01TESTUSERID000000000".to_string(),
        reply_to_post_id: None,
        content: trimmed.to_string(),
        effective_visibility: "public".to_string(),
        state: "published".to_string(),
        version: 1,
    };

    repo.posts.lock().unwrap().push(post.clone());
    Ok((StatusCode::CREATED, Json(post)))
}

pub fn app(repo: InMemoryRepo) -> Router {
    Router::new()
        .route("/api/v1/posts", post(create_post))
        .with_state(repo)
}

#[tokio::main]
async fn main() {
    let repo = InMemoryRepo::default();
    let app = app(repo);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn create_post_returns_201() {
        let repo = InMemoryRepo::default();
        let app = app(repo);

        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/posts")
            .header("content-type", "application/json")
            .header("idempotency-key", "demo-key-1")
            .body(Body::from(r#"{"content":"こんにちは MIZ"}"#))
            .unwrap();

        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);

        let body = res.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["content"], "こんにちは MIZ");
    }
}
```

### SvelteKit TypeScript テンプレート

```ts
// apps/web/src/routes/mock/api/v1/posts/+server.ts
import { json, error, type RequestHandler } from '@sveltejs/kit';

type PostDto = {
  id: string;
  authorId: string;
  replyToPostId: string | null;
  content: string;
  effectiveVisibility: 'public' | 'followers';
  state: 'published';
  version: number;
};

const posts: PostDto[] = [];
const segmenter = new Intl.Segmenter('ja', { granularity: 'grapheme' });

function graphemeCount(value: string): number {
  return Array.from(segmenter.segment(value)).length;
}

export const POST: RequestHandler = async ({ request }) => {
  const idempotencyKey = request.headers.get('idempotency-key');
  if (!idempotencyKey) {
    throw error(428, {
      type: 'https://api.miz.local/problems/idempotency_required',
      title: 'Precondition Required',
      status: 428,
      detail: 'Idempotency-Key header is required',
      code: 'idempotency_required'
    });
  }

  const body = (await request.json()) as { content?: string };
  const content = (body.content ?? '').trim();

  if (!content) {
    throw error(400, {
      type: 'https://api.miz.local/problems/content_empty',
      title: 'Bad Request',
      status: 400,
      detail: 'content must not be blank',
      code: 'content_empty'
    });
  }

  if (graphemeCount(content) > 500 || new TextEncoder().encode(content).length > 8000) {
    throw error(400, {
      type: 'https://api.miz.local/problems/content_too_long',
      title: 'Bad Request',
      status: 400,
      detail: 'content exceeds allowed size',
      code: 'content_too_long'
    });
  }

  const post: PostDto = {
    id: 'p_01WEBMOCKPOST000000000',
    authorId: 'u_01WEBMOCKUSER000000000',
    replyToPostId: null,
    content,
    effectiveVisibility: 'public',
    state: 'published',
    version: 1
  };

  posts.push(post);
  return json(post, { status: 201 });
};
```

```ts
// apps/web/src/routes/mock/api/v1/posts/post.server.test.ts
import { describe, expect, it } from 'vitest';
import { POST } from './+server';

describe('mock POST /api/v1/posts', () => {
  it('returns 201 for valid content', async () => {
    const request = new Request('http://localhost/mock/api/v1/posts', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        'idempotency-key': 'demo-key-1'
      },
      body: JSON.stringify({ content: 'こんにちは MIZ' })
    });

    const response = await POST({ request } as Parameters<typeof POST>[0]);
    expect(response.status).toBe(201);

    const json = await response.json();
    expect(json.content).toBe('こんにちは MIZ');
  });
});
```

### SQL マイグレーション

```sql
-- apps/api/migrations/0001_mvp.sql
CREATE TABLE users (
  id VARCHAR(22) PRIMARY KEY,
  handle VARCHAR(24) NOT NULL,
  display_name VARCHAR(50) NOT NULL,
  bio TEXT NOT NULL DEFAULT '',
  privacy VARCHAR(16) NOT NULL CHECK (privacy IN ('public', 'private')),
  status VARCHAR(16) NOT NULL DEFAULT 'active',
  version BIGINT NOT NULL DEFAULT 1,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX users_handle_unique_ci ON users (LOWER(handle));

CREATE TABLE handle_aliases (
  old_handle VARCHAR(24) PRIMARY KEY,
  user_id VARCHAR(22) NOT NULL REFERENCES users(id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE user_private_attributes (
  user_id VARCHAR(22) PRIMARY KEY REFERENCES users(id),
  birth_date DATE NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE user_consents (
  user_id VARCHAR(22) NOT NULL REFERENCES users(id),
  document_type VARCHAR(32) NOT NULL,
  document_version VARCHAR(32) NOT NULL,
  accepted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, document_type, document_version)
);

CREATE TABLE password_credentials (
  user_id VARCHAR(22) PRIMARY KEY REFERENCES users(id),
  password_hash TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE sessions (
  id VARCHAR(22) PRIMARY KEY,
  user_id VARCHAR(22) NOT NULL REFERENCES users(id),
  token_hash TEXT NOT NULL,
  device_name TEXT NOT NULL DEFAULT '',
  last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  idle_expires_at TIMESTAMPTZ NOT NULL,
  absolute_expires_at TIMESTAMPTZ NOT NULL,
  revoked_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX sessions_user_idx ON sessions (user_id, last_seen_at DESC);
CREATE INDEX sessions_expiry_idx ON sessions (idle_expires_at, absolute_expires_at);

CREATE TABLE posts (
  id VARCHAR(22) PRIMARY KEY,
  author_id VARCHAR(22) NOT NULL REFERENCES users(id),
  reply_to_post_id VARCHAR(22) REFERENCES posts(id),
  content TEXT NOT NULL,
  effective_visibility VARCHAR(16) NOT NULL CHECK (effective_visibility IN ('public', 'followers')),
  state VARCHAR(16) NOT NULL CHECK (state IN ('published', 'deleted', 'tombstone')),
  version BIGINT NOT NULL DEFAULT 1,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  deleted_at TIMESTAMPTZ
);

CREATE INDEX posts_author_created_idx ON posts (author_id, created_at DESC, id DESC);
CREATE INDEX posts_reply_created_idx ON posts (reply_to_post_id, created_at ASC, id ASC);

CREATE TABLE follow_relationships (
  id VARCHAR(22) PRIMARY KEY,
  follower_id VARCHAR(22) NOT NULL REFERENCES users(id),
  followee_id VARCHAR(22) NOT NULL REFERENCES users(id),
  status VARCHAR(16) NOT NULL CHECK (status IN ('pending', 'accepted', 'rejected', 'cancelled', 'removed')),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT follow_no_self CHECK (follower_id <> followee_id),
  UNIQUE (follower_id, followee_id)
);

CREATE INDEX follow_followee_status_idx
  ON follow_relationships (followee_id, status, created_at DESC);

CREATE TABLE idempotency_keys (
  user_id VARCHAR(22) NOT NULL REFERENCES users(id),
  endpoint TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  request_hash TEXT NOT NULL,
  response_status INT NOT NULL,
  response_body JSONB NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  expires_at TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (user_id, endpoint, idempotency_key)
);
```

このマイグレーションは PostgreSQL の標準的な `CREATE TABLE` / `CREATE INDEX` 機能だけで成立する最小構成です。必要な複合インデックスと一意制約を先に作っておくと、timeline や handle lookup、session list のパスが単純になります。citeturn0search2turn0search6turn0search10

## 配備・CI・四週間計画

### docker-compose.yml

```yaml
version: "3.9"

services:
  db:
    image: postgres:17
    environment:
      POSTGRES_USER: miz
      POSTGRES_PASSWORD: miz
      POSTGRES_DB: miz
    ports:
      - "5432:5432"
    volumes:
      - pgdata:/var/lib/postgresql/data

  api:
    build: ./apps/api
    environment:
      RUST_LOG: info
      DATABASE_URL: postgres://miz:miz@db:5432/miz
      APP_BASE_URL: http://localhost:5173
      SESSION_SECRET: change-me
      GOOGLE_CLIENT_ID: change-me
      GOOGLE_CLIENT_SECRET: change-me
    depends_on:
      - db
    ports:
      - "8080:8080"

  web:
    build: ./apps/web
    environment:
      PUBLIC_API_BASE_URL: http://localhost:8080
    depends_on:
      - api
    ports:
      - "5173:5173"

volumes:
  pgdata:
```

### GitHub Actions

```yaml
# .github/workflows/ci.yml
name: ci

on:
  push:
  pull_request:

jobs:
  api:
    runs-on: ubuntu-latest
    services:
      db:
        image: postgres:17
        env:
          POSTGRES_USER: miz
          POSTGRES_PASSWORD: miz
          POSTGRES_DB: miz
        ports:
          - 5432:5432
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cd apps/api && cargo fmt --check
      - run: cd apps/api && cargo test
        env:
          DATABASE_URL: postgres://miz:miz@localhost:5432/miz

  web:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: "22"
      - run: cd apps/web && npm ci
      - run: cd apps/web && npm run test
      - run: cd apps/web && npm run build
```

### デプロイチェックリスト

| 項目 | 内容 |
|---|---|
| 環境変数 | `DATABASE_URL`, `SESSION_SECRET`, `APP_BASE_URL`, `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`, `SMTP_*` を設定 |
| Cookie | 本番 HTTPS で `Secure`、`HttpOnly`、`SameSite=Lax` を付与 |
| CSRF | state-changing request 全てに CSRF token 検証 |
| OpenAPI | `openapi/openapi.yaml` を正本に固定し、差分を CI で検知 |
| DB migration | アプリ起動前に migrate 実行、初期化順序を固定 |
| ログ | `requestId`、userId、endpoint、status を構造化出力。トークンや Cookie は出さない |
| Health check | `/healthz` と `/readyz` を分離 |
| リリース | staging で migration + smoke test 成功後に production |
| ロールバック | 直前コンテナイメージに戻す手順と後方互換 migration ルールを文書化 |

### テスト・CI コマンド

```bash
# API
cd apps/api
cargo fmt --check
cargo test

# Web
cd ../web
npm ci
npm run test
npm run build

# Local full stack
cd ../..
docker compose up --build
```

### 四週間ガントチャート

```mermaid
gantt
    title MIZ MVP 4週間計画
    dateFormat  YYYY-MM-DD
    section Week 1
    API契約/OpenAPI/Problem Details固定     :a1, 2026-07-27, 3d
    DBスキーマ/migration/ID方式            :a2, 2026-07-29, 3d
    section Week 2
    username/password 登録・ログイン        :b1, 2026-08-03, 4d
    セッション管理/プロフィール/公開設定   :b2, 2026-08-06, 2d
    section Week 3
    短文投稿/Reply/Idempotency             :c1, 2026-08-10, 3d
    Follow申請/承認/一覧API               :c2, 2026-08-13, 2d
    section Week 4
    ホームタイムライン/keyset pagination   :d1, 2026-08-17, 3d
    Docker/CI/E2E/リリース確認            :d2, 2026-08-20, 3d
```

Docker Compose は multi-container アプリを単一 YAML で定義・起動する公式手段であり、Compose file reference で services, networks, volumes を定義できます。GitHub Actions は CI/CD のための YAML workflow を `.github/workflows` に置くのが公式の運用です。SvelteKit 側は server routes と load/actions を使うと、frontend と backend の並行実装がしやすくなります。citeturn0search3turn0search7turn1search0turn1search4turn1search12turn0search1turn0search9

## 未確定事項と前提

この CSV から**確定できること**と**未指定のため仮定したこと**は分けて扱うべきです。確定できるのは、**Rust/Axum + SvelteKit + PostgreSQL + OpenAPI + Cookie セッション** という方向性、`TYPE=task` と `TYPE=note` の結びつき、`@mvp` と priority 1 foundation が MVP の中心であることです。逆に、**最低利用年齢の具体値、利用規約/プライバシーポリシーの文書本文と version、クラウドベンダー、画像/動画ストレージ、予約語一覧、監査ログ保持期間**は CSV 単体では未指定です。

本計画で採った前提は次のとおりです。  
**前提一**として、CSV の一部 note はセル内に省略記号があり全文が確認できないため、**見えている要件だけを authoritative requirements として採用**しました。  
**前提二**として、認証方式は CSV が既に明示しているため、ユーザープロンプトの「未指定なら no-auth」は**今回適用しません**。  
**前提三**として、ホスティングは未指定なので、**containerized deployment** を維持し、Docker Compose をローカル基準にしました。  
**前提四**として、`短文投稿` task の note に添付の痕跡はあるものの、専用メディア task が post-MVP にあるため、**MVP は本文のみ**に切り、メディアは後続へ送ります。  
**前提五**として、SSE は foundation に触れられている一方で realtime task が post-MVP のため、**ホームタイムラインは polling で先に出し、SSE は後続追加**としました。  
**前提六**として、メール認証を廃止し、**MVPではusername + password登録を正規経路**としました。Passkeyはpost-MVPです。

以上を踏まえると、Codex に直接 handoff すべき最小指示は、**まず OpenAPI と SQL migration を固定し、次に registration/auth/session、続いて profile/privacy、post/reply、follow/timeline の順で実装する**ことです。Problem Details、Cookie 属性、CSRF、Argon2id、Docker Compose、GitHub Actions はいずれも一次資料が揃っており、ここで示した plan はすべて公式仕様・公式ドキュメントの範囲内で素直に実装できます。citeturn2search0turn4search1turn4search5turn3search1turn2search1turn0search3turn1search0