use miz_api::infrastructure;

#[tokio::main]
async fn main() {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = infrastructure::database(&url)
        .await
        .expect("database must be reachable");
    let removed = infrastructure::purge_expired_report_evidence(&pool)
        .await
        .expect("report evidence retention must succeed");
    let removed_audit = infrastructure::purge_expired_audit_logs(&pool)
        .await
        .expect("audit retention must succeed");
    let expired_restrictions = infrastructure::expire_temporary_restrictions(&pool)
        .await
        .expect("restriction expiry must succeed");
    let purged_accounts = infrastructure::purge_expired_accounts(&pool)
        .await
        .expect("account purge must succeed");
    println!(
        "removedReportEvidence={removed} removedAuditLogs={removed_audit} expiredRestrictions={expired_restrictions} purgedAccounts={purged_accounts}"
    );
}
