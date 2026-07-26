<script lang="ts">
  import { onMount } from 'svelte';
  import { createApiClient, type components } from '@miz/api-client';

  type User = components['schemas']['User'];
  type Post = components['schemas']['Post'];
  type Session = components['schemas']['Session'];
  type Relationship = components['schemas']['FollowRelationship'];
  type Report = components['schemas']['Report'];
  type ReportReason = components['schemas']['ReportReason'];

  const api = createApiClient();
  const reasons: ReportReason[] = ['spam', 'harassment', 'hatefulContent', 'violence', 'sexualContent', 'illegalOrDangerousTrade', 'personalInformation', 'copyright', 'other'];
  let user: User | null = null;
  let sessions: Session[] = [];
  let followers: Relationship[] = [];
  let following: Relationship[] = [];
  let requests: Relationship[] = [];
  let selectedPost: Post | null = null;
  let replies: Post[] = [];
  let selectedReport: Report | null = null;
  let busy = false;
  let message = '';
  let error = '';

  let handle = '';
  let displayName = '';
  let bio = '';
  let privacy: 'public' | 'private' = 'public';
  let targetUserId = '';
  let postId = '';
  let postContent = '';
  let replyContent = '';
  let reportReason: ReportReason = 'spam';
  let reportExplanation = '';
  let reportId = '';
  let deletionPassword = '';
  let recoveryUsername = '';
  let recoveryPassword = '';
  let appealActionId = '';
  let appealExplanation = '';

  const csrf = () => document.cookie.split('; ').find((value) => value.startsWith('__Host-miz_csrf='))?.split('=')[1] ?? '';
  const mutationHeaders = () => ({ 'x-csrf-token': csrf() });
  const fail = (detail?: string) => { error = detail ?? '処理に失敗しました。'; message = ''; };
  const done = (text: string) => { message = text; error = ''; };

  async function load() {
    const result = await api.GET('/api/v1/users/me');
    if (!result.data) return;
    user = result.data;
    handle = user.handle;
    displayName = user.displayName;
    bio = user.bio;
    privacy = user.privacy;
    await refreshAccountData();
    const query = new URLSearchParams(location.search);
    targetUserId = query.get('user') ?? '';
    postId = query.get('post') ?? '';
    if (postId) await getPost();
  }

  async function refreshAccountData() {
    if (!user) return;
    const [sessionResult, followerResult, followingResult, requestResult] = await Promise.all([
      api.GET('/api/v1/sessions'),
      api.GET('/api/v1/users/{userId}/followers', { params: { path: { userId: user.id } } }),
      api.GET('/api/v1/users/{userId}/following', { params: { path: { userId: user.id } } }),
      api.GET('/api/v1/follow-requests')
    ]);
    sessions = sessionResult.data ?? [];
    followers = followerResult.data?.items ?? [];
    following = followingResult.data?.items ?? [];
    requests = requestResult.data?.items ?? [];
  }

  async function updateProfile() {
    if (!user) return;
    busy = true;
    const result = await api.PATCH('/api/v1/users/me', {
      params: { header: { 'If-Match': `"${user.version}"` } },
      headers: mutationHeaders(), body: { handle, displayName, bio, privacy }
    });
    busy = false;
    if (result.data) { user = result.data; done('プロフィールを更新しました。'); }
    else fail(result.error?.detail);
  }

  async function revokeSession(id?: string) {
    busy = true;
    const result = id
      ? await api.DELETE('/api/v1/sessions/{sessionId}', { params: { path: { sessionId: id } }, headers: mutationHeaders() })
      : await api.DELETE('/api/v1/sessions', { headers: mutationHeaders() });
    busy = false;
    if (result.error) fail(result.error.detail);
    else { done(id ? 'セッションを取り消しました。' : '全セッションを取り消しました。'); if (id) await refreshAccountData(); else user = null; }
  }

  async function relationship(action: 'follow' | 'unfollow' | 'block' | 'unblock' | 'mute' | 'unmute') {
    if (!targetUserId) return;
    busy = true;
    const options = { params: { path: { targetUserId } }, headers: mutationHeaders() };
    const result = action === 'follow' ? await api.PUT('/api/v1/users/{targetUserId}/follow', options)
      : action === 'unfollow' ? await api.DELETE('/api/v1/users/{targetUserId}/follow', options)
      : action === 'block' ? await api.POST('/api/v1/users/{targetUserId}/block', options)
      : action === 'unblock' ? await api.DELETE('/api/v1/users/{targetUserId}/block', options)
      : action === 'mute' ? await api.POST('/api/v1/users/{targetUserId}/mute', options)
      : await api.DELETE('/api/v1/users/{targetUserId}/mute', options);
    busy = false;
    if (result.error) fail(result.error.detail); else { done(`${action} を実行しました。`); await refreshAccountData(); }
  }

  async function decideRequest(id: string, decision: 'accept' | 'reject') {
    busy = true;
    const result = decision === 'accept'
      ? await api.POST('/api/v1/follow-requests/{relationshipId}/accept', { params: { path: { relationshipId: id } }, headers: mutationHeaders() })
      : await api.POST('/api/v1/follow-requests/{relationshipId}/reject', { params: { path: { relationshipId: id } }, headers: mutationHeaders() });
    busy = false;
    if (result.error) fail(result.error.detail); else { done(`申請を${decision === 'accept' ? '承認' : '拒否'}しました。`); await refreshAccountData(); }
  }

  async function getPost() {
    if (!postId) return;
    busy = true;
    const [postResult, repliesResult] = await Promise.all([
      api.GET('/api/v1/posts/{postId}', { params: { path: { postId } } }),
      api.GET('/api/v1/posts/{postId}/replies', { params: { path: { postId }, query: { limit: 100 } } })
    ]);
    busy = false;
    if (postResult.data) { selectedPost = postResult.data; postContent = postResult.data.content ?? ''; replies = repliesResult.data?.items ?? []; done('投稿を読み込みました。'); }
    else fail(postResult.error?.detail);
  }

  async function updatePost() {
    if (!selectedPost) return;
    busy = true;
    const result = await api.PATCH('/api/v1/posts/{postId}', {
      params: { path: { postId: selectedPost.id }, header: { 'If-Match': `"${selectedPost.version}"` } },
      headers: mutationHeaders(), body: { content: postContent }
    });
    busy = false;
    if (result.data) { selectedPost = result.data; done('投稿を編集しました。'); } else fail(result.error?.detail);
  }

  async function deletePost() {
    if (!selectedPost || !confirm('この投稿を削除しますか？')) return;
    busy = true;
    const result = await api.DELETE('/api/v1/posts/{postId}', {
      params: { path: { postId: selectedPost.id }, header: { 'If-Match': `"${selectedPost.version}"` } }, headers: mutationHeaders()
    });
    busy = false;
    if (result.error) fail(result.error.detail); else { selectedPost = null; replies = []; done('投稿を削除しました。'); }
  }

  async function createReply() {
    if (!selectedPost || !replyContent.trim()) return;
    busy = true;
    const result = await api.POST('/api/v1/posts/{postId}/replies', {
      params: { path: { postId: selectedPost.id }, header: { 'Idempotency-Key': crypto.randomUUID() } },
      headers: mutationHeaders(), body: { content: replyContent }
    });
    busy = false;
    if (result.data) { replies = [...replies, result.data]; replyContent = ''; done('返信しました。'); } else fail(result.error?.detail);
  }

  async function createReport() {
    if (!selectedPost) return;
    busy = true;
    const result = await api.POST('/api/v1/posts/{postId}/reports', {
      params: { path: { postId: selectedPost.id } }, headers: mutationHeaders(),
      body: { reason: reportReason, ...(reportExplanation ? { explanation: reportExplanation } : {}) }
    });
    busy = false;
    if (result.data) { selectedReport = result.data; reportId = result.data.id; done('報告を送信しました。'); } else fail(result.error?.detail);
  }

  async function getReport() {
    if (!reportId) return;
    const result = await api.GET('/api/v1/reports/{reportId}', { params: { path: { reportId } } });
    if (result.data) { selectedReport = result.data; reportReason = result.data.reason; reportExplanation = result.data.explanation ?? ''; done('報告を読み込みました。'); }
    else fail(result.error?.detail);
  }

  async function updateReport() {
    if (!selectedReport) return;
    const result = await api.PATCH('/api/v1/reports/{reportId}', {
      params: { path: { reportId: selectedReport.id }, header: { 'If-Match': `"${selectedReport.version}"` } }, headers: mutationHeaders(),
      body: { reason: reportReason, explanation: reportExplanation }
    });
    if (result.data) { selectedReport = result.data; done('報告を更新しました。'); } else fail(result.error?.detail);
  }

  async function deleteAccount() {
    if (!deletionPassword || !confirm('アカウントを削除待ち状態にします。続けますか？')) return;
    const result = await api.POST('/api/v1/users/me/deletion-requests', { headers: mutationHeaders(), body: { password: deletionPassword } });
    if (result.data) { user = null; done(`削除を受け付けました。復元期限: ${new Date(result.data.restoreUntil).toLocaleString('ja-JP')}`); }
    else fail(result.error?.detail);
  }

  async function recover(kind: 'cancel' | 'restore') {
    const options = { headers: mutationHeaders(), body: { username: recoveryUsername, password: recoveryPassword } };
    const result = kind === 'cancel'
      ? await api.POST('/api/v1/users/me/deletion-requests/current/cancel', options)
      : await api.POST('/api/v1/users/me/deletion-requests/current/restore', options);
    if (result.data) done(kind === 'cancel' ? '削除申請を取り消しました。' : 'アカウントを復元しました。'); else fail(result.error?.detail);
  }

  async function createAppeal() {
    const result = await api.POST('/api/v1/appeals', { body: { username: recoveryUsername, password: recoveryPassword, actionId: appealActionId, explanation: appealExplanation } });
    if (result.data) done(`異議申し立てを受け付けました。ID: ${result.data.id}`); else fail(result.error?.detail);
  }

  onMount(load);
</script>

<svelte:head><title>機能管理 — MIZ</title><meta name="description" content="MIZのプロフィール、投稿、つながり、セッションを管理します。" /></svelte:head>

<header><a class="brand" href="/">MIZ</a><nav><a href="/">タイムライン</a><a href="/manage" aria-current="page">機能管理</a><a href="/admin">運営</a></nav></header>
<main>
  <div class="title"><p>CONTROL DESK</p><h1>機能管理</h1><span>Backendで利用可能なユーザー機能をまとめています。</span></div>
  {#if message}<p class="notice" role="status">{message}</p>{/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}

  {#if user}
    <section><h2>プロフィール</h2><form onsubmit={(e) => { e.preventDefault(); updateProfile(); }}>
      <label>ユーザー名<input bind:value={handle} required /></label><label>表示名<input bind:value={displayName} required /></label>
      <label>自己紹介<textarea bind:value={bio} rows="3"></textarea></label><label>公開範囲<select bind:value={privacy}><option value="public">公開</option><option value="private">非公開</option></select></label>
      <button disabled={busy}>更新する</button></form></section>

    <section><h2>つながり・安全</h2><label>対象ユーザーID<input bind:value={targetUserId} placeholder="ユーザーID" /></label>
      <div class="actions"><button onclick={() => relationship('follow')}>フォロー</button><button onclick={() => relationship('unfollow')}>解除</button><button onclick={() => relationship('mute')}>ミュート</button><button onclick={() => relationship('unmute')}>解除</button><button class="danger" onclick={() => relationship('block')}>ブロック</button><button onclick={() => relationship('unblock')}>解除</button></div>
      <div class="columns"><div><h3>フォロー申請</h3>{#each requests as item}<p class="row"><code>{item.followerId}</code><span><button onclick={() => decideRequest(item.id, 'accept')}>承認</button><button onclick={() => decideRequest(item.id, 'reject')}>拒否</button></span></p>{:else}<p class="muted">申請はありません。</p>{/each}</div>
      <div><h3>フォロワー ({followers.length})</h3>{#each followers as item}<p><code>{item.followerId}</code></p>{/each}<h3>フォロー中 ({following.length})</h3>{#each following as item}<p><code>{item.followeeId}</code></p>{/each}</div></div>
    </section>

    <section><h2>投稿・返信・報告</h2><div class="inline"><input bind:value={postId} placeholder="投稿ID" /><button onclick={getPost}>読み込む</button></div>
      {#if selectedPost}<article><small>{selectedPost.id} / version {selectedPost.version}</small><textarea bind:value={postContent} rows="4"></textarea><div class="actions"><button onclick={updatePost}>編集</button><button class="danger" onclick={deletePost}>削除</button></div></article>
        <h3>返信</h3>{#each replies as reply}<article class="reply"><small>{reply.authorId}</small><p>{reply.content}</p></article>{:else}<p class="muted">返信はありません。</p>{/each}
        <form onsubmit={(e) => { e.preventDefault(); createReply(); }}><textarea bind:value={replyContent} rows="2" placeholder="返信を入力" required></textarea><button>返信する</button></form>
        <h3>この投稿を報告</h3><label>理由<select bind:value={reportReason}>{#each reasons as reason}<option value={reason}>{reason}</option>{/each}</select></label><label>説明<textarea bind:value={reportExplanation} rows="2"></textarea></label><button onclick={createReport}>報告する</button>
      {/if}
      <h3>送信済み報告</h3><div class="inline"><input bind:value={reportId} placeholder="報告ID" /><button onclick={getReport}>読み込む</button></div>
      {#if selectedReport}<p>状態: <strong>{selectedReport.status}</strong> / version {selectedReport.version}</p><button onclick={updateReport}>理由・説明を更新</button>{/if}
    </section>

    <section><h2>ログイン中の端末</h2>{#each sessions as session}<p class="row"><span>{session.deviceName}<small>{new Date(session.lastSeenAt).toLocaleString('ja-JP')} {session.current ? '（現在）' : ''}</small></span><button onclick={() => revokeSession(session.id)}>取り消す</button></p>{/each}<button class="danger" onclick={() => revokeSession()}>すべてログアウト</button></section>

    <section class="danger-zone"><h2>アカウント削除</h2><p>申請後30日間は復元できます。申請すると全セッションが終了します。</p><label>現在のパスワード<input type="password" bind:value={deletionPassword} /></label><button class="danger" onclick={deleteAccount}>削除を申請</button></section>
  {:else}
    <section><h2>ログインが必要です</h2><p><a href="/#login">ログインまたは新規登録</a>してください。削除申請の取り消し・復元、異議申し立ては下から行えます。</p></section>
  {/if}

  <section><h2>削除申請の取り消し・復元</h2><label>ユーザー名<input bind:value={recoveryUsername} /></label><label>パスワード<input type="password" bind:value={recoveryPassword} /></label><div class="actions"><button onclick={() => recover('cancel')}>申請を取り消す</button><button onclick={() => recover('restore')}>復元する</button></div></section>
  <section><h2>モデレーションへの異議申し立て</h2><label>措置ID<input bind:value={appealActionId} /></label><label>説明<textarea bind:value={appealExplanation} rows="3"></textarea></label><button onclick={createAppeal}>送信する</button></section>
</main>

<style>
  :global(*){box-sizing:border-box}:global(body){margin:0;color:#17223a;background:#f4f1e8;font-family:"Hiragino Kaku Gothic ProN","Yu Gothic",sans-serif}:global(button),:global(input),:global(textarea),:global(select){font:inherit}
  header{height:76px;display:flex;align-items:center;justify-content:space-between;padding:0 clamp(20px,5vw,72px);border-top:6px solid #1646d8;border-bottom:1px solid #17223a}.brand{color:inherit;font:bold 24px Georgia,serif;letter-spacing:.18em;text-decoration:none}nav{display:flex;gap:22px}nav a{color:inherit;font-size:12px;text-decoration:none}nav a[aria-current]{color:#1646d8;font-weight:700}
  main{width:min(1000px,calc(100% - 32px));margin:56px auto 100px}.title{margin-bottom:42px}.title p{color:#1646d8;font:bold 10px Georgia,serif;letter-spacing:.2em}.title h1{margin:10px 0;font:500 clamp(38px,6vw,68px) "Yu Mincho",serif}.title span,.muted{font-size:12px;opacity:.62}
  section{margin:0 0 24px;padding:28px;border:1px solid #17223a;background:rgba(255,255,255,.3)}h2{margin:0 0 24px;font:500 25px "Yu Mincho",serif}h3{margin-top:30px;font-size:13px}form{display:grid;gap:16px}label{display:grid;gap:8px;margin:12px 0;font-size:11px;font-weight:700}input,textarea,select{width:100%;border:1px solid rgba(23,34,58,.45);background:#fffdf6;padding:11px;color:inherit}textarea{resize:vertical}button{width:fit-content;border:1px solid #17223a;background:transparent;padding:9px 14px;cursor:pointer;font-size:11px;font-weight:700}button:hover{color:white;background:#1646d8;border-color:#1646d8}button:disabled{opacity:.5}.danger{color:#b52d18;border-color:#b52d18}.danger:hover{background:#b52d18}.danger-zone{border-color:#b52d18}
  .actions,.inline{display:flex;flex-wrap:wrap;gap:8px;align-items:center}.inline input{flex:1}.columns{display:grid;grid-template-columns:1fr 1fr;gap:28px}.row{display:flex;align-items:center;justify-content:space-between;gap:20px;padding:10px 0;border-bottom:1px solid rgba(23,34,58,.15)}.row small{display:block;margin-top:5px;opacity:.58}code{overflow-wrap:anywhere;font-size:10px}article{margin:18px 0;padding:18px;border-left:4px solid #1646d8;background:#fffdf6}.reply{border-left-width:1px}.notice,.error{padding:13px 16px;border-left:4px solid #1646d8;background:#fffdf6}.error{border-color:#b52d18}
  @media(max-width:650px){header{height:auto;align-items:flex-start;gap:18px;padding:20px}nav{flex-wrap:wrap;justify-content:flex-end;gap:12px}.columns{grid-template-columns:1fr}section{padding:20px}.row{align-items:flex-start;flex-direction:column}}
</style>
