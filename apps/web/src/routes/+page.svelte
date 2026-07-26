<script lang="ts">
  import { onMount } from 'svelte';
  import { createApiClient, type components } from '@miz/api-client';

  type User = components['schemas']['User'];
  type Post = components['schemas']['Post'];

  const api = createApiClient();
  const date = new Intl.DateTimeFormat('ja-JP', { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });

  let user: User | null = null;
  let posts: Post[] = [];
  let nextCursor: string | null = null;
  let loading = true;
  let sending = false;
  let authMode: 'register' | 'login' = 'register';
  let username = '';
  let password = '';
  let displayName = '';
  let birthDate = '';
  let content = '';
  let notice = '';
  let errorMessage = '';

  const csrfToken = () => document.cookie
    .split('; ')
    .find((cookie) => cookie.startsWith('__Host-miz_csrf='))
    ?.split('=')[1] ?? '';

  async function loadApp() {
    loading = true;
    const me = await api.GET('/api/v1/users/me');
    if (me.data) {
      user = me.data;
      await loadTimeline();
    }
    loading = false;
  }

  async function loadTimeline(cursor?: string) {
    const result = await api.GET('/api/v1/timelines/home', {
      params: { query: { limit: 30, cursor } }
    });
    if (result.data) {
      posts = cursor ? [...posts, ...result.data.items] : result.data.items;
      nextCursor = result.data.nextCursor;
    } else {
      errorMessage = result.error?.detail ?? 'タイムラインを読み込めませんでした。';
    }
  }

  async function authenticate() {
    sending = true;
    notice = '';
    errorMessage = '';
    const result = authMode === 'register'
      ? await api.POST('/api/v1/registrations', {
          body: { username, password, displayName, birthDate }
        })
      : await api.POST('/api/v1/auth/login', {
          body: { username, password }
        });
    sending = false;
    if (result.data) {
      user = result.data;
      password = '';
      await loadTimeline();
    } else {
      errorMessage = result.error?.detail ?? '認証できませんでした。';
    }
  }

  async function publish() {
    if (!content.trim()) return;
    sending = true;
    errorMessage = '';
    const result = await api.POST('/api/v1/posts', {
      params: { header: { 'Idempotency-Key': crypto.randomUUID() } },
      headers: { 'x-csrf-token': csrfToken() },
      body: { content }
    });
    sending = false;
    if (result.data) {
      posts = [result.data, ...posts];
      content = '';
      notice = '投稿しました。';
    } else {
      errorMessage = result.error?.detail ?? '投稿できませんでした。';
    }
  }

  async function logout() {
    await api.DELETE('/api/v1/sessions/current', {
      headers: { 'x-csrf-token': csrfToken() }
    });
    user = null;
    posts = [];
  }

  onMount(loadApp);
</script>

<svelte:head>
  <title>MIZ — 近くの声を、静かに。</title>
  <meta name="description" content="短い言葉でつながる、シンプルなソーシャルスペース。" />
</svelte:head>

<div class="page-shell">
  <header class="masthead">
    <a class="brand" href="/" aria-label="MIZ ホーム">
      <span class="brand-mark">M</span>
      <span>MIZ</span>
    </a>
    <p class="tagline">近くの声を、静かに。</p>
    {#if user}
      <div class="header-actions"><a class="text-button" href="/manage">機能管理</a><button class="text-button" type="button" onclick={logout}>ログアウト</button></div>
    {:else}
      <a class="text-button" href="#login">はじめる</a>
    {/if}
  </header>

  {#if loading}
    <main class="loading" aria-live="polite">
      <span></span><span></span><span></span>
      <p>声を集めています</p>
    </main>
  {:else if !user}
    <main class="welcome">
      <section class="hero" aria-labelledby="hero-title">
        <p class="issue">MIZ / ISSUE 001</p>
        <h1 id="hero-title">話すほどでもないことを、<em>話せる場所。</em></h1>
        <p class="hero-copy">短い言葉を置いて、気になる人をフォローする。余計な機能をそぎ落とした、小さなソーシャルスペースです。</p>
        <div class="principles" aria-label="MIZの特徴">
          <span>01<br /><strong>500文字まで</strong></span>
          <span>02<br /><strong>広告なし</strong></span>
          <span>03<br /><strong>自分のペースで</strong></span>
        </div>
      </section>

      <section class="login-card" id="login" aria-labelledby="login-title">
        <div class="card-number">01</div>
        <div class="auth-tabs" aria-label="認証方法">
          <button type="button" class:active={authMode === 'register'} onclick={() => { authMode = 'register'; errorMessage = ''; }}>新規登録</button>
          <button type="button" class:active={authMode === 'login'} onclick={() => { authMode = 'login'; errorMessage = ''; }}>ログイン</button>
        </div>
        <h2 id="login-title">{authMode === 'register' ? 'MIZをはじめる' : 'おかえりなさい'}</h2>
        <p>{authMode === 'register' ? 'ユーザー名とパスワードだけで、すぐに始められます。' : '登録したユーザー名とパスワードを入力してください。'}</p>
        <form onsubmit={(event) => { event.preventDefault(); authenticate(); }}>
          <label for="username">ユーザー名</label>
          <div class="handle-input"><span>@</span><input id="username" autocomplete="username" pattern="[a-z0-9_]+" minlength="3" maxlength="24" placeholder="miz_user" bind:value={username} required /></div>
          <label for="password">パスワード</label>
          <input id="password" type="password" autocomplete={authMode === 'register' ? 'new-password' : 'current-password'} minlength="12" maxlength="128" placeholder="12文字以上" bind:value={password} required />
          {#if authMode === 'register'}
            <label for="display-name">表示名</label>
            <input id="display-name" autocomplete="name" maxlength="50" placeholder="水野 ミズ" bind:value={displayName} required />
            <label for="birth-date">生年月日</label>
            <input id="birth-date" type="date" autocomplete="bday" bind:value={birthDate} required />
            <label class="consent"><input type="checkbox" required /> <span>ローカルMVPの利用条件とプライバシー方針に同意します</span></label>
          {/if}
          <button class="primary" type="submit" disabled={sending}>{sending ? '処理中…' : authMode === 'register' ? 'アカウントを作成' : 'ログイン'}<span aria-hidden="true">→</span></button>
        </form>
        {#if errorMessage}<p class="message error" role="alert">{errorMessage}</p>{/if}
        <small>パスワードはArgon2idでハッシュ化され、平文では保存されません。</small>
      </section>
    </main>
  {:else}
    <main class="app-grid">
      <aside class="profile-panel">
        <div class="avatar" aria-hidden="true">{user.displayName.slice(0, 1)}</div>
        <p class="eyebrow">YOUR PROFILE</p>
        <h2>{user.displayName}</h2>
        <p class="handle">@{user.handle}</p>
        {#if user.bio}<p class="bio">{user.bio}</p>{/if}
        <dl>
          <div><dt>公開範囲</dt><dd>{user.privacy === 'public' ? '公開' : 'フォロワーのみ'}</dd></div>
          <div><dt>参加</dt><dd>{new Date(user.createdAt).getFullYear()}</dd></div>
        </dl>
      </aside>

      <section class="feed" aria-labelledby="feed-title">
        <div class="feed-heading">
          <div><p class="eyebrow">FOLLOWING / LATEST</p><h1 id="feed-title">タイムライン</h1></div>
          <button class="refresh" type="button" onclick={() => loadTimeline()} aria-label="タイムラインを更新">↻</button>
        </div>

        <form class="composer" onsubmit={(event) => { event.preventDefault(); publish(); }}>
          <label for="post">いま、何を考えていますか？</label>
          <textarea id="post" rows="3" bind:value={content} placeholder="短い言葉を置いてみる…"></textarea>
          <div>
            <span>テキストのみ・500文字まで</span>
            <button class="post-button" type="submit" disabled={sending || !content.trim()}>{sending ? '送信中…' : '投稿する'} <span aria-hidden="true">→</span></button>
          </div>
        </form>

        {#if notice}<p class="message success" role="status">{notice}</p>{/if}
        {#if errorMessage}<p class="message error" role="alert">{errorMessage}</p>{/if}

        <div class="post-list">
          {#each posts as post (post.id)}
            <article class="post">
              <div class="post-avatar" aria-hidden="true">{post.authorId === user.id ? user.displayName.slice(0, 1) : '・'}</div>
              <div class="post-body">
                <header>
                  <strong>{post.authorId === user.id ? user.displayName : 'MIZ ユーザー'}</strong>
                  <span>{post.authorId === user.id ? `@${user.handle}` : `@${post.authorId.slice(0, 8)}`}</span>
                  <time datetime={post.createdAt}>{date.format(new Date(post.createdAt))}</time>
                </header>
                <p>{post.content}</p>
                <footer><span>{post.editedAt ? '編集済み' : '公開'}</span><span>{post.effectiveVisibility === 'public' ? '○ すべての人' : '◐ フォロワー'}</span><a href={`/manage?post=${post.id}&user=${post.authorId}`}>返信・操作</a></footer>
              </div>
            </article>
          {:else}
            <div class="empty">
              <span>〽</span>
              <h3>まだ静かです。</h3>
              <p>最初のひとことを投稿してみましょう。</p>
            </div>
          {/each}
        </div>

        {#if nextCursor}
          <button class="more" type="button" onclick={() => loadTimeline(nextCursor ?? undefined)}>以前の投稿を読む</button>
        {/if}
      </section>

      <aside class="note-panel">
        <p class="eyebrow">MIZ NOTE</p>
        <blockquote>「うまく言えない」も、ひとつの言葉です。</blockquote>
        <p>ここでは、速く反応する必要はありません。読みたいときに読み、話したいときに話してください。</p>
        <span class="note-mark">水</span>
      </aside>
    </main>
  {/if}
</div>

<style>
  :global(*) { box-sizing: border-box; }
  :global(html) { scroll-behavior: smooth; }
  :global(body) { margin: 0; color: #17223a; background: #f4f1e8; font-family: "Hiragino Kaku Gothic ProN", "Yu Gothic", sans-serif; }
  :global(button), :global(input), :global(textarea) { font: inherit; }
  :global(button), :global(a) { -webkit-tap-highlight-color: transparent; }

  :global(body::before) { content: ''; position: fixed; inset: 0; z-index: -1; pointer-events: none; opacity: .2; background-image: url("data:image/svg+xml,%3Csvg viewBox='0 0 180 180' xmlns='http://www.w3.org/2000/svg'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='.9' numOctaves='3' stitchTiles='stitch'/%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)' opacity='.18'/%3E%3C/svg%3E"); }
  .page-shell { min-height: 100vh; border-top: 6px solid #1646d8; }
  .masthead { height: 84px; display: grid; grid-template-columns: 1fr auto 1fr; align-items: center; border-bottom: 1px solid rgba(23,34,58,.25); padding: 0 clamp(22px, 5vw, 72px); }
  .brand { display: flex; width: fit-content; align-items: center; gap: 12px; color: inherit; text-decoration: none; font: 800 24px/1 Georgia, serif; letter-spacing: .18em; }
  .brand-mark { display: grid; place-items: center; width: 32px; height: 32px; color: #f4f1e8; background: #1646d8; font-size: 19px; letter-spacing: 0; transform: rotate(-3deg); }
  .tagline { font-family: "Yu Mincho", "Hiragino Mincho ProN", serif; font-size: 13px; letter-spacing: .16em; }
  .header-actions { justify-self: end; display: flex; gap: 18px; }
  .text-button { justify-self: end; border: 0; border-bottom: 1px solid currentColor; color: inherit; background: none; padding: 5px 0; text-decoration: none; cursor: pointer; font-size: 12px; font-weight: 700; }

  .welcome { min-height: calc(100vh - 90px); display: grid; grid-template-columns: minmax(0, 1.45fr) minmax(330px, .55fr); }
  .hero { position: relative; display: flex; flex-direction: column; justify-content: center; overflow: hidden; padding: clamp(60px, 9vw, 130px) clamp(26px, 8vw, 140px); border-right: 1px solid rgba(23,34,58,.25); }
  .hero::after { content: '水'; position: absolute; right: -4vw; bottom: -14vw; color: transparent; -webkit-text-stroke: 1px rgba(22,70,216,.18); font: 400 min(42vw, 620px)/1 "Yu Mincho", serif; transform: rotate(7deg); pointer-events: none; }
  .issue, .eyebrow { margin: 0 0 22px; color: #1646d8; font: 800 10px/1.2 Georgia, serif; letter-spacing: .22em; }
  .hero h1 { max-width: 850px; margin: 0; font: 500 clamp(45px, 6.3vw, 96px)/1.08 "Yu Mincho", "Hiragino Mincho ProN", serif; letter-spacing: -.055em; }
  .hero h1 em { display: block; color: #1646d8; font-style: normal; }
  .hero-copy { max-width: 570px; margin: 40px 0 55px; font: 400 15px/2 "Yu Mincho", serif; }
  .principles { display: flex; gap: clamp(24px, 5vw, 72px); position: relative; z-index: 1; font: 700 9px/2 Georgia, serif; letter-spacing: .14em; color: #e64b2a; }
  .principles strong { color: #17223a; font: 600 12px/1.4 "Yu Gothic", sans-serif; letter-spacing: .04em; }
  .login-card { align-self: center; margin: 40px clamp(24px, 4vw, 64px); padding: clamp(30px, 4vw, 52px); border: 1px solid #17223a; background: rgba(255,255,255,.28); box-shadow: 12px 12px 0 #1646d8; }
  .card-number { color: #e64b2a; font: 700 11px Georgia, serif; }
  .auth-tabs { display: flex; gap: 18px; margin: 26px 0 0; border-bottom: 1px solid rgba(23,34,58,.2); }
  .auth-tabs button { border: 0; border-bottom: 3px solid transparent; color: rgba(23,34,58,.5); background: transparent; padding: 0 0 10px; cursor: pointer; font-size: 11px; font-weight: 700; }
  .auth-tabs button.active { border-color: #1646d8; color: #17223a; }
  .login-card h2 { margin: 28px 0 12px; font: 500 30px "Yu Mincho", serif; }
  .login-card > p { font-size: 13px; line-height: 1.8; }
  .login-card form { margin: 35px 0 24px; }
  label { display: block; margin-bottom: 10px; font-size: 11px; font-weight: 700; letter-spacing: .08em; }
  input { width: 100%; border: 0; border-bottom: 1px solid #17223a; outline: 0; background: transparent; padding: 14px 2px; border-radius: 0; }
  input:focus { border-color: #1646d8; box-shadow: 0 2px #1646d8; }
  .login-card form label:not(:first-child) { margin-top: 20px; }
  .handle-input { display: flex; align-items: center; border-bottom: 1px solid #17223a; }
  .handle-input span { color: #1646d8; }
  .handle-input input { border: 0; }
  .consent { display: flex !important; align-items: flex-start; gap: 9px; font-size: 10px; line-height: 1.6; cursor: pointer; }
  .consent input { width: 15px; height: 15px; flex: 0 0 auto; margin: 1px 0 0; accent-color: #1646d8; }
  .primary { width: 100%; display: flex; justify-content: space-between; margin-top: 22px; border: 0; color: white; background: #17223a; padding: 17px 18px; cursor: pointer; font-size: 12px; font-weight: 700; }
  .primary:hover, .post-button:hover { background: #1646d8; }
  button:disabled { opacity: .5; cursor: wait; }
  .login-card small { display: block; font-size: 9px; line-height: 1.7; opacity: .58; }

  .loading { min-height: calc(100vh - 90px); display: flex; align-items: center; justify-content: center; gap: 7px; }
  .loading span { width: 7px; height: 7px; background: #1646d8; animation: pulse 1s infinite alternate; }
  .loading span:nth-child(2) { animation-delay: .2s; }.loading span:nth-child(3) { animation-delay: .4s; }
  .loading p { margin-left: 12px; font-size: 11px; letter-spacing: .12em; }
  @keyframes pulse { to { transform: translateY(-8px); background: #e64b2a; } }

  .app-grid { display: grid; grid-template-columns: minmax(190px, 260px) minmax(420px, 680px) minmax(190px, 280px); justify-content: center; gap: clamp(24px, 4vw, 64px); padding: 64px clamp(24px, 5vw, 72px) 100px; }
  .profile-panel, .note-panel { position: sticky; top: 42px; align-self: start; }
  .avatar { width: 76px; height: 76px; display: grid; place-items: center; margin-bottom: 28px; border-radius: 50%; color: #f4f1e8; background: #1646d8; font: 500 34px "Yu Mincho", serif; box-shadow: 5px 5px 0 #e64b2a; }
  .profile-panel h2 { margin: 0; font: 500 24px "Yu Mincho", serif; }
  .handle { margin: 6px 0 20px; color: #1646d8; font: 12px Georgia, serif; }
  .bio, .note-panel > p { font: 12px/1.9 "Yu Mincho", serif; }
  dl { margin-top: 30px; border-top: 1px solid rgba(23,34,58,.25); }
  dl div { display: flex; justify-content: space-between; padding: 12px 0; border-bottom: 1px solid rgba(23,34,58,.25); font-size: 10px; }
  dt { opacity: .55; } dd { margin: 0; font-weight: 700; }
  .feed { min-width: 0; }
  .feed-heading { display: flex; justify-content: space-between; align-items: end; margin-bottom: 28px; }
  .feed-heading .eyebrow { margin-bottom: 9px; }
  .feed-heading h1 { margin: 0; font: 500 34px "Yu Mincho", serif; }
  .refresh { width: 38px; height: 38px; border: 1px solid #17223a; border-radius: 50%; color: inherit; background: transparent; cursor: pointer; font-size: 20px; }
  .refresh:hover { color: white; background: #1646d8; transform: rotate(45deg); }
  .composer { margin-bottom: 26px; border: 1px solid #17223a; background: rgba(255,255,255,.34); padding: 24px; box-shadow: 7px 7px 0 rgba(22,70,216,.9); }
  textarea { width: 100%; resize: vertical; border: 0; outline: 0; color: inherit; background: transparent; font: 16px/1.8 "Yu Mincho", serif; }
  .composer > div { display: flex; justify-content: space-between; align-items: center; padding-top: 14px; border-top: 1px solid rgba(23,34,58,.2); }
  .composer > div > span { font-size: 9px; opacity: .5; }
  .post-button { border: 0; color: white; background: #17223a; padding: 11px 16px; cursor: pointer; font-size: 11px; font-weight: 700; }
  .post-button span { margin-left: 16px; }
  .post-list { border-top: 1px solid #17223a; }
  .post { display: grid; grid-template-columns: 42px 1fr; gap: 16px; padding: 28px 4px; border-bottom: 1px solid rgba(23,34,58,.22); animation: reveal .35s both; }
  @keyframes reveal { from { opacity: 0; transform: translateY(8px); } }
  .post-avatar { width: 38px; height: 38px; display: grid; place-items: center; border: 1px solid #17223a; border-radius: 50%; color: #1646d8; font-family: "Yu Mincho", serif; }
  .post-body header { display: flex; align-items: baseline; gap: 8px; font-size: 11px; }
  .post-body header span, time { color: rgba(23,34,58,.48); }
  .post-body time { margin-left: auto; font-size: 9px; }
  .post-body > p { white-space: pre-wrap; margin: 14px 0 22px; font: 15px/1.85 "Yu Mincho", serif; overflow-wrap: anywhere; }
  .post-body footer { display: flex; gap: 18px; color: rgba(23,34,58,.5); font-size: 9px; }
  .post-body footer a { margin-left: auto; color: #1646d8; font-weight: 700; }
  .empty { padding: 80px 20px; text-align: center; }
  .empty > span { color: #1646d8; font-size: 38px; }.empty h3 { font: 500 22px "Yu Mincho", serif; }.empty p { font-size: 11px; opacity: .6; }
  .more { width: 100%; margin-top: 28px; border: 1px solid #17223a; color: inherit; background: transparent; padding: 15px; cursor: pointer; font-size: 11px; font-weight: 700; }
  .more:hover { color: white; background: #17223a; }
  .note-panel { padding-top: 80px; border-top: 4px solid #e64b2a; }
  .note-panel blockquote { margin: 22px 0; font: 500 25px/1.7 "Yu Mincho", serif; }
  .note-mark { display: block; margin-top: 35px; color: #1646d8; font: 60px "Yu Mincho", serif; opacity: .15; }
  .message { margin: 16px 0; padding: 12px 14px; font-size: 11px; line-height: 1.6; }
  .success { border-left: 3px solid #1646d8; background: rgba(22,70,216,.08); }
  .error { border-left: 3px solid #e64b2a; background: rgba(230,75,42,.08); }

  @media (max-width: 960px) {
    .app-grid { grid-template-columns: minmax(0, 680px); }
    .profile-panel, .note-panel { display: none; }
    .welcome { grid-template-columns: 1fr; }
    .hero { border-right: 0; border-bottom: 1px solid rgba(23,34,58,.25); }
    .login-card { width: min(500px, calc(100% - 48px)); justify-self: center; }
  }
  @media (max-width: 600px) {
    .masthead { height: 70px; grid-template-columns: 1fr 1fr; padding: 0 20px; }
    .tagline { display: none; }
    .welcome { min-height: calc(100vh - 76px); }
    .hero { padding: 64px 24px 54px; }
    .hero h1 { font-size: clamp(39px, 12vw, 58px); }
    .hero-copy { margin: 30px 0 38px; }
    .principles { gap: 18px; }
    .login-card { margin: 52px 24px 70px; padding: 28px; box-shadow: 8px 8px 0 #1646d8; }
    .app-grid { padding: 40px 16px 80px; }
    .composer { padding: 18px; box-shadow: 5px 5px 0 #1646d8; }
    .post { grid-template-columns: 34px 1fr; gap: 11px; }
    .post-avatar { width: 32px; height: 32px; }
    .post-body header { flex-wrap: wrap; }
    .post-body time { flex-basis: 100%; margin: 2px 0 0; }
  }
  @media (prefers-reduced-motion: reduce) { *, *::before, *::after { scroll-behavior: auto !important; animation: none !important; transition: none !important; } }
</style>
