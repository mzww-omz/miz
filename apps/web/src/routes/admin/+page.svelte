<script lang="ts">
  import { onMount } from 'svelte';
  import { createApiClient, type components } from '@miz/api-client';

  type Operator = components['schemas']['Operator'];
  type OperatorSession = components['schemas']['OperatorSession'];
  type AdminReport = components['schemas']['AdminReport'];
  type AdminAppeal = components['schemas']['AdminAppeal'];
  type Role = Operator['roles'][number];

  const api = createApiClient();
  const allowedRoles: Role[] = ['support', 'moderator', 'seniorModerator', 'administrator', 'auditor'];
  let operator: Operator | null = null;
  let sessions: OperatorSession[] = [];
  let reports: AdminReport[] = [];
  let selectedReport: AdminReport | null = null;
  let appeals: AdminAppeal[] = [];
  let output = '';
  let message = '';
  let error = '';
  let busy = false;

  let username = '', password = '', totpCode = '', recoveryCode = '';
  let enrollmentToken = '', enrollmentCode = '';
  let reportStatus: '' | 'received' | 'inReview' | 'actioned' | 'noAction' = '';
  let reportId = '', reviewStatus: 'inReview' | 'actioned' | 'noAction' = 'inReview', reviewReason = '', removeContent = false;
  let newUsername = '', newPassword = '', roleText = 'support';
  let operatorId = '', roleReason = '';
  let accountId = '', restrictionKind: 'featureRestriction' | 'temporarySuspension' | 'permanentSuspension' = 'featureRestriction', restrictionFeature = '', restrictionExpiry = '', restrictionReason = '', restrictionReportId = '';
  let appealStatus: '' | 'pending' | 'upheld' | 'overturned' = '', appealId = '', appealDecision: 'upheld' | 'overturned' = 'upheld', appealReason = '', appealVersion = 1;
  let auditTargetType: 'post' | 'user' | 'report' | 'operator' | 'appeal' = 'user', auditTargetId = '', auditFrom = '', auditTo = '';

  const csrf = () => document.cookie.split('; ').find((value) => value.startsWith('__Host-miz_operator_csrf='))?.split('=')[1] ?? '';
  const headers = () => ({ 'x-csrf-token': csrf() });
  const done = (text: string, data?: unknown) => { message = text; error = ''; output = data === undefined ? '' : JSON.stringify(data, null, 2); };
  const fail = (detail?: string) => { error = detail ?? '処理に失敗しました。'; message = ''; };
  const roles = (): Role[] => roleText.split(',').map((value) => value.trim()).filter((value): value is Role => allowedRoles.includes(value as Role));

  async function loadOperator() {
    const result = await api.GET('/api/v1/admin/me');
    if (result.data) { operator = result.data; await loadSessions(); }
  }

  async function login() {
    busy = true;
    const result = await api.POST('/api/v1/admin/auth/login', { body: { username, password, ...(totpCode ? { totpCode } : {}), ...(recoveryCode ? { recoveryCode } : {}) } });
    busy = false;
    if (result.data) { operator = result.data; password = ''; done('運営アカウントでログインしました。'); await loadSessions(); } else fail(result.error?.detail);
  }

  async function logout() {
    const result = await api.DELETE('/api/v1/admin/auth/logout', { headers: headers() });
    if (result.error) fail(result.error.detail); else { operator = null; done('ログアウトしました。'); }
  }

  async function enrollMfa() {
    const result = await api.POST('/api/v1/admin/auth/mfa/enroll', { body: { enrollmentToken, totpCode: enrollmentCode } });
    if (result.error) fail(result.error.detail); else done('MFAを登録しました。ログインしてください。');
  }

  async function confirmMfa() {
    const result = await api.POST('/api/v1/admin/auth/mfa/challenge', { headers: headers(), body: { ...(totpCode ? { totpCode } : {}), ...(recoveryCode ? { recoveryCode } : {}) } });
    if (result.error) fail(result.error.detail); else { done('MFAを再確認しました。'); await loadOperator(); }
  }

  async function recoveryCodes() {
    const result = await api.POST('/api/v1/admin/auth/mfa/recovery-codes', { headers: headers() });
    if (result.data) done('新しいリカバリーコードです。安全な場所に保存してください。', result.data); else fail(result.error?.detail);
  }

  async function loadSessions() {
    const result = await api.GET('/api/v1/admin/sessions');
    if (result.data) sessions = result.data; else fail(result.error?.detail);
  }

  async function revokeSession(id: string) {
    const result = await api.DELETE('/api/v1/admin/sessions/{sessionId}', { params: { path: { sessionId: id } }, headers: headers() });
    if (result.error) fail(result.error.detail); else { done('セッションを取り消しました。'); await loadSessions(); }
  }

  async function createOperator() {
    const result = await api.POST('/api/v1/admin/operators', { headers: headers(), body: { username: newUsername, password: newPassword, roles: roles() } });
    if (result.data) done('運営アカウントを作成しました。登録情報は一度だけ安全に共有してください。', result.data); else fail(result.error?.detail);
  }

  async function replaceRoles() {
    const result = await api.PUT('/api/v1/admin/operators/{operatorId}/roles', { params: { path: { operatorId } }, headers: headers(), body: { roles: roles(), reason: roleReason } });
    if (result.error) fail(result.error.detail); else done('ロールを更新しました。');
  }

  async function loadReports() {
    const result = await api.GET('/api/v1/admin/reports', { params: { query: { limit: 100, ...(reportStatus ? { status: reportStatus } : {}) } } });
    if (result.data) { reports = result.data.items; done(`${reports.length}件の報告を読み込みました。`); } else fail(result.error?.detail);
  }

  async function getReport(id = reportId) {
    if (!id) return;
    const result = await api.GET('/api/v1/admin/reports/{reportId}', { params: { path: { reportId: id } } });
    if (result.data) { selectedReport = result.data; reportId = result.data.id; done('報告と証拠スナップショットを読み込みました。'); } else fail(result.error?.detail);
  }

  async function reviewReport() {
    if (!selectedReport) return;
    const result = await api.PATCH('/api/v1/admin/reports/{reportId}', {
      params: { path: { reportId: selectedReport.id }, header: { 'If-Match': `"${selectedReport.version}"` } }, headers: headers(),
      body: { status: reviewStatus, reason: reviewReason, removeContent }
    });
    if (result.data) { selectedReport = result.data; done('報告を更新しました。'); await loadReports(); } else fail(result.error?.detail);
  }

  async function getAccount() {
    const result = await api.GET('/api/v1/admin/users/{userId}', { params: { path: { userId: accountId } } });
    if (result.data) done('アカウントを読み込みました。', result.data); else fail(result.error?.detail);
  }

  async function restrictAccount() {
    const result = await api.POST('/api/v1/admin/users/{userId}/restrictions', {
      params: { path: { userId: accountId } }, headers: headers(), body: {
        kind: restrictionKind, feature: restrictionFeature || null, expiresAt: restrictionExpiry ? new Date(restrictionExpiry).toISOString() : null,
        reason: restrictionReason, reportId: restrictionReportId || null
      }
    });
    if (result.data) done('措置を適用しました。', result.data); else fail(result.error?.detail);
  }

  async function loadAppeals() {
    const result = await api.GET('/api/v1/admin/appeals', { params: { query: { limit: 100, ...(appealStatus ? { status: appealStatus } : {}) } } });
    if (result.data) { appeals = result.data; done(`${appeals.length}件の異議申し立てを読み込みました。`); } else fail(result.error?.detail);
  }

  async function reviewAppeal() {
    const result = await api.PATCH('/api/v1/admin/appeals/{appealId}', {
      params: { path: { appealId }, header: { 'If-Match': `"${appealVersion}"` } }, headers: headers(), body: { status: appealDecision, reason: appealReason }
    });
    if (result.data) { done('異議申し立てを審査しました。', result.data); await loadAppeals(); } else fail(result.error?.detail);
  }

  async function loadAudit() {
    if (!auditTargetId || !auditFrom || !auditTo) { fail('対象ID、開始、終了を入力してください。'); return; }
    const result = await api.GET('/api/v1/admin/audit-logs', { params: { query: {
      limit: 100, targetType: auditTargetType, targetId: auditTargetId,
      from: new Date(auditFrom).toISOString(), to: new Date(auditTo).toISOString()
    } } });
    if (result.data) done(`${result.data.length}件の監査ログを読み込みました。`, result.data); else fail(result.error?.detail);
  }

  async function checkSystem() {
    const [health, readiness] = await Promise.all([fetch('/healthz'), fetch('/readyz')]);
    done('システム状態を確認しました。', { health: health.status, readiness: readiness.status });
  }

  onMount(loadOperator);
</script>

<svelte:head><title>運営コンソール — MIZ</title><meta name="robots" content="noindex,nofollow" /></svelte:head>
<header><a class="brand" href="/">MIZ</a><nav><a href="/">タイムライン</a><a href="/manage">機能管理</a><a href="/admin" aria-current="page">運営</a></nav></header>
<main>
  <div class="title"><p>OPERATOR CONSOLE</p><h1>運営コンソール</h1><span>運営専用アカウントとMFAが必要です。</span></div>
  {#if message}<p class="notice" role="status">{message}</p>{/if}{#if error}<p class="error" role="alert">{error}</p>{/if}{#if output}<pre>{output}</pre>{/if}

  {#if !operator}
    <div class="grid"><section><h2>ログイン</h2><form onsubmit={(e)=>{e.preventDefault();login();}}><label>ユーザー名<input bind:value={username} required /></label><label>パスワード<input type="password" bind:value={password} required /></label><label>TOTP（任意）<input inputmode="numeric" bind:value={totpCode} /></label><label>リカバリーコード（任意）<input bind:value={recoveryCode} /></label><button disabled={busy}>ログイン</button></form></section>
    <section><h2>初回MFA登録</h2><form onsubmit={(e)=>{e.preventDefault();enrollMfa();}}><label>登録トークン<input bind:value={enrollmentToken} required /></label><label>TOTP<input inputmode="numeric" bind:value={enrollmentCode} required /></label><button>登録する</button></form></section></div>
  {:else}
    <section class="operator"><div><small>LOGIN AS</small><h2>{operator.username}</h2><p>{operator.roles.join(' / ')} · MFA {operator.recentMfa ? '確認済み' : '再確認が必要'}</p></div><button onclick={logout}>ログアウト</button></section>
    <div class="grid">
      <section><h2>MFA・セッション</h2><label>TOTP<input bind:value={totpCode} /></label><label>リカバリーコード<input bind:value={recoveryCode} /></label><div class="actions"><button onclick={confirmMfa}>再確認</button><button onclick={recoveryCodes}>コード再発行</button></div>{#each sessions as session}<p class="row"><span>{session.id}<small>{session.current ? '現在 · ' : ''}{new Date(session.lastSeenAt).toLocaleString('ja-JP')}</small></span><button onclick={()=>revokeSession(session.id)}>取消</button></p>{/each}</section>
      <section><h2>システム</h2><p>ヘルスチェック、Readiness、OpenAPI契約を確認します。</p><div class="actions"><button onclick={checkSystem}>状態確認</button><a class="button" href="/openapi.json" target="_blank">OpenAPI</a></div></section>
    </div>

    <section><h2>報告キュー</h2><div class="toolbar"><select bind:value={reportStatus}><option value="">すべて</option><option>received</option><option>inReview</option><option>actioned</option><option>noAction</option></select><button onclick={loadReports}>読み込む</button><input bind:value={reportId} placeholder="報告ID" /><button onclick={()=>getReport()}>IDで取得</button></div>{#each reports as report}<button class="list-item" onclick={()=>getReport(report.id)}><strong>{report.reason}</strong><span>{report.status} · {report.targetType} {report.targetId}</span></button>{/each}
      {#if selectedReport}<article><small>証拠 / version {selectedReport.version}</small><p>{selectedReport.evidenceContent}</p><code>{selectedReport.id}</code></article><label>状態<select bind:value={reviewStatus}><option>inReview</option><option>actioned</option><option>noAction</option></select></label><label>判断理由<textarea bind:value={reviewReason}></textarea></label><label class="check"><input type="checkbox" bind:checked={removeContent} />コンテンツを削除</label><button onclick={reviewReport}>審査を保存</button>{/if}
    </section>

    <div class="grid"><section><h2>アカウント措置</h2><label>ユーザーID<input bind:value={accountId} /></label><button onclick={getAccount}>基本情報を取得</button><label>措置<select bind:value={restrictionKind}><option>featureRestriction</option><option>temporarySuspension</option><option>permanentSuspension</option></select></label><label>対象機能<input bind:value={restrictionFeature} /></label><label>期限<input type="datetime-local" bind:value={restrictionExpiry} /></label><label>理由<textarea bind:value={restrictionReason}></textarea></label><label>関連報告ID<input bind:value={restrictionReportId} /></label><button class="danger" onclick={restrictAccount}>措置を適用</button></section>
      <section><h2>運営アカウント作成</h2><label>ユーザー名<input bind:value={newUsername} /></label><label>初期パスワード<input type="password" bind:value={newPassword} /></label><label>ロール（カンマ区切り）<input bind:value={roleText} /></label><button onclick={createOperator}>作成</button><hr /><h3>ロール変更</h3><label>運営ID<input bind:value={operatorId} /></label><label>理由<input bind:value={roleReason} /></label><button onclick={replaceRoles}>変更</button></section></div>

    <section><h2>異議申し立て</h2><div class="toolbar"><select bind:value={appealStatus}><option value="">すべて</option><option>pending</option><option>upheld</option><option>overturned</option></select><button onclick={loadAppeals}>読み込む</button></div>{#each appeals as appeal}<button class="list-item" onclick={()=>{appealId=appeal.id;appealVersion=appeal.version}}><strong>{appeal.status}</strong><span>{appeal.id} · action {appeal.actionId} · version {appeal.version}</span></button>{/each}<label>異議ID<input bind:value={appealId} /></label><label>version<input type="number" min="1" bind:value={appealVersion} /></label><label>判断<select bind:value={appealDecision}><option>upheld</option><option>overturned</option></select></label><label>理由<textarea bind:value={appealReason}></textarea></label><button onclick={reviewAppeal}>審査を保存</button></section>

    <section><h2>監査ログ</h2><div class="filters"><label>対象種別<select bind:value={auditTargetType}><option>user</option><option>post</option><option>report</option><option>operator</option><option>appeal</option></select></label><label>対象ID<input bind:value={auditTargetId} /></label><label>開始<input type="datetime-local" bind:value={auditFrom} /></label><label>終了<input type="datetime-local" bind:value={auditTo} /></label></div><button onclick={loadAudit}>読み込む</button></section>
  {/if}
</main>

<style>
  :global(*){box-sizing:border-box}:global(body){margin:0;color:#18221d;background:#edf0ea;font-family:"Hiragino Kaku Gothic ProN","Yu Gothic",sans-serif}:global(button),:global(input),:global(textarea),:global(select){font:inherit}header{height:76px;display:flex;align-items:center;justify-content:space-between;padding:0 clamp(20px,5vw,72px);border-top:6px solid #1c593d;border-bottom:1px solid #18221d}.brand{color:inherit;font:bold 24px Georgia,serif;letter-spacing:.18em;text-decoration:none}nav{display:flex;gap:22px}nav a{color:inherit;font-size:12px;text-decoration:none}nav a[aria-current]{color:#1c593d;font-weight:700}main{width:min(1100px,calc(100% - 32px));margin:56px auto 100px}.title{margin-bottom:42px}.title p{color:#1c593d;font:bold 10px Georgia,serif;letter-spacing:.2em}.title h1{margin:10px 0;font:500 clamp(38px,6vw,64px) "Yu Mincho",serif}.title span{font-size:12px;opacity:.65}.grid{display:grid;grid-template-columns:1fr 1fr;gap:20px}section{margin-bottom:20px;padding:26px;border:1px solid #18221d;background:rgba(255,255,255,.4)}h2{margin:0 0 22px;font:500 24px "Yu Mincho",serif}h3{font-size:13px}label{display:grid;gap:7px;margin:12px 0;font-size:11px;font-weight:700}input,textarea,select{width:100%;border:1px solid rgba(24,34,29,.4);background:#fff;padding:10px;color:inherit}textarea{min-height:80px;resize:vertical}button,.button{display:inline-block;width:fit-content;border:1px solid #18221d;color:inherit;background:transparent;padding:9px 13px;cursor:pointer;font-size:11px;font-weight:700;text-decoration:none}button:hover,.button:hover{color:white;background:#1c593d;border-color:#1c593d}.danger{color:#a52d20;border-color:#a52d20}.danger:hover{background:#a52d20}.actions,.toolbar{display:flex;align-items:center;flex-wrap:wrap;gap:8px}.toolbar>*{width:auto}.toolbar input{flex:1}.operator{display:flex;justify-content:space-between;align-items:center;border-top:5px solid #1c593d}.operator h2{margin:5px 0}.operator p{font-size:11px}.row{display:flex;justify-content:space-between;gap:12px;padding:10px 0;border-bottom:1px solid rgba(24,34,29,.16);font:10px monospace}.row small{display:block;margin-top:5px}.list-item{display:flex;width:100%;justify-content:space-between;gap:15px;margin-top:-1px;text-align:left}.list-item span{font-weight:400;opacity:.7}article{margin:18px 0;padding:18px;border-left:4px solid #1c593d;background:white}article p{white-space:pre-wrap}.check{display:flex;grid-template-columns:auto 1fr;align-items:center}.check input{width:auto}.filters{display:grid;grid-template-columns:repeat(4,1fr);gap:10px}pre{max-height:320px;overflow:auto;padding:18px;color:#ddf9e8;background:#14241b;font:11px/1.6 monospace}.notice,.error{padding:13px 16px;border-left:4px solid #1c593d;background:white}.error{border-color:#a52d20}hr{margin:28px 0;border:0;border-top:1px solid rgba(24,34,29,.2)}
  @media(max-width:700px){header{height:auto;align-items:flex-start;gap:18px;padding:20px}nav{flex-wrap:wrap;justify-content:flex-end;gap:12px}.grid,.filters{grid-template-columns:1fr}section{padding:20px}.list-item,.operator{align-items:flex-start;flex-direction:column}}
</style>
