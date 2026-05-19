//! Interactive smoke-test for the OS-cron pipeline.
//!
//! Run with:
//!   ATA_BIN=/tmp/fake-ata.sh cargo run -p codex-scheduling --example os_cron_demo -- create
//!   cargo run -p codex-scheduling --example os_cron_demo -- list
//!   cargo run -p codex-scheduling --example os_cron_demo -- log <task_id>
//!   cargo run -p codex-scheduling --example os_cron_demo -- delete <task_id>
//!
//! Does NOT launch a real ata. With `ATA_BIN=/tmp/fake-ata.sh`, each firing
//! runs the fake script, which just appends a timestamp + the prompt to
//! ~/.ata/cron/<task_id>.log. This validates the entire crontab pipeline
//! without burning API tokens.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use codex_scheduling::CronJob;
use codex_scheduling::os_cron;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("help");

    match cmd {
        "create" => create(),
        "list" => list(),
        "log" => log(&args),
        "delete" => delete(&args),
        _ => {
            eprintln!(
                "Usage:\n  os_cron_demo create\n  os_cron_demo list\n  os_cron_demo log <task_id>\n  os_cron_demo delete <task_id>"
            );
            std::process::exit(1);
        }
    }
}

fn create() {
    let bin = os_cron::resolve_ata_binary().expect("resolve ata binary (set ATA_BIN)");
    println!("Using ATA_BIN = {}", bin.display());

    // Fire every minute. Prompt is the body the fake script will see on stdin.
    let job = CronJob::new(
        "0 * * * * *".to_string(),
        "hello from os_cron_demo".to_string(),
    )
    .expect("valid cron expr");

    println!("Inserting job {} ...", job.id);
    os_cron::insert(&job).expect("insert into crontab");

    let dir = os_cron::data_dir().unwrap();
    println!("✓ Inserted. Files:");
    println!(
        "    prompt: {}",
        dir.join(format!("{}.prompt", job.id)).display()
    );
    println!(
        "    log:    {}",
        dir.join(format!("{}.log", job.id)).display()
    );
    println!();
    println!("Next steps:");
    println!("  1. Wait 60-120 seconds for cron to fire.");
    println!(
        "  2. cargo run -p codex-scheduling --example os_cron_demo -- log {}",
        job.id
    );
    println!(
        "  3. cargo run -p codex-scheduling --example os_cron_demo -- delete {}",
        job.id
    );
}

fn list() {
    let entries = os_cron::list().expect("read crontab");
    if entries.is_empty() {
        println!("(no ata-managed cron entries)");
        return;
    }
    println!(
        "Found {} ata cron entr{}:",
        entries.len(),
        if entries.len() == 1 { "y" } else { "ies" }
    );
    for e in entries {
        let next = os_cron::next_fire_after_now_five_field(&e.cron_expr_five_field)
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|| "?".to_string());
        println!(
            "  {} | schedule={} | next_fire={} | prompt={:?}",
            e.task_id, e.cron_expr_five_field, next, e.prompt
        );
    }
}

fn log(args: &[String]) {
    let id = args.get(2).expect("usage: log <task_id>");
    let path = os_cron::data_dir().unwrap().join(format!("{id}.log"));
    println!("--- {} ---", path.display());
    match std::fs::read_to_string(&path) {
        Ok(s) if s.is_empty() => {
            println!("(log empty — cron hasn't fired yet, or fired with no output)")
        }
        Ok(s) => print!("{s}"),
        Err(e) => println!("(no log file: {e})"),
    }
}

fn delete(args: &[String]) {
    let id = args.get(2).expect("usage: delete <task_id>");
    let task_id = codex_scheduling::TaskId::from(id.clone());
    let removed = os_cron::delete(&task_id).expect("delete from crontab");
    if removed {
        println!("✓ Removed crontab entry and prompt file for {id}");
        println!("  (log file kept at ~/.ata/cron/{id}.log — delete manually if you want)");
    } else {
        println!("✗ No entry found for {id}");
    }
}
