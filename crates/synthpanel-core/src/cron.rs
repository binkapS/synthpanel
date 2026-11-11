//! Cron schedule parsing and the job scheduler.
//!
//! The panel parses standard five-field crontab expressions itself so it can
//! answer "when does this job run next?" without shelling out, and so the UI
//! can validate an expression before it is written to a crontab.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::SqlitePool;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// A parsed five-field crontab expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    minutes: BTreeSet<u32>,
    hours: BTreeSet<u32>,
    days: BTreeSet<u32>,
    months: BTreeSet<u32>,
    weekdays: BTreeSet<u32>,
    /// Whether the day-of-month and day-of-week fields are combined with OR,
    /// which is how Vixie cron behaves.
    dom_restricted: bool,
    dow_restricted: bool,
}

/// A human readable description of a schedule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleDescription {
    pub expression: String,
    pub summary: String,
    pub next_run: Option<String>,
}

impl Schedule {
    /// Parse a five-field expression: minute hour day-of-month month day-of-week.
    pub fn parse(expression: &str) -> Result<Self> {
        let fields: Vec<&str> = expression.split_whitespace().collect();
        if fields.len() != 5 {
            bail!(
                "a cron expression needs five fields (minute hour day month weekday), got {}",
                fields.len()
            );
        }
        let minutes = parse_field(fields[0], 0, 59, &["*", ","])?;
        let hours = parse_field(fields[1], 0, 23, &["*", ","])?;
        let days = parse_field(fields[2], 1, 31, &["*", ","])?;
        let months = parse_field(fields[3], 1, 12, &["*", ","])?;
        let weekdays = parse_weekday(fields[4])?;

        Ok(Self {
            minutes,
            hours,
            days,
            months,
            weekdays,
            dom_restricted: fields[2].trim() != "*",
            dow_restricted: fields[4].trim() != "*",
        })
    }

    /// Whether the schedule fires at the given minute.
    pub fn matches(&self, moment: DateTime<Utc>) -> bool {
        if !self.minutes.contains(&moment.minute())
            || !self.hours.contains(&moment.hour())
            || !self.months.contains(&moment.month())
        {
            return false;
        }
        let day_matches = self.days.contains(&moment.day());
        let weekday_matches = self
            .weekdays
            .contains(&moment.weekday().num_days_from_sunday());
        match (self.dom_restricted, self.dow_restricted) {
            (true, true) => day_matches || weekday_matches,
            (true, false) => day_matches,
            (false, true) => weekday_matches,
            (false, false) => true,
        }
    }

    /// The next moment the schedule fires after `after`.
    pub fn next_run(&self, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let mut candidate = after.with_second(0)?.with_nanosecond(0)? + Duration::minutes(1);
        // Look ahead at most two years; beyond that the schedule is unusable.
        let limit = after + Duration::days(730);
        while candidate <= limit {
            if self.matches(candidate) {
                return Some(candidate);
            }
            candidate += Duration::minutes(1);
            // Skip a whole hour quickly when the hour does not match.
            if !self.hours.contains(&candidate.hour()) {
                candidate = candidate.with_minute(0)?.with_second(0)? + Duration::hours(1);
            }
        }
        None
    }

    /// A short human readable summary.
    pub fn summary(&self) -> String {
        if self.minutes.len() == 1
            && self.hours.len() == 1
            && self.days.len() == 1
            && self.months.len() == 1
            && self.weekdays.len() == 1
            && self.dom_restricted
            && self.dow_restricted
        {
            return format!(
                "at {}:{:02} on day {} of month {}",
                self.hours.iter().next().unwrap_or(&0),
                self.minutes.iter().next().unwrap_or(&0),
                self.days.iter().next().unwrap_or(&1),
                self.months.iter().next().unwrap_or(&1)
            );
        }
        if self.minutes.len() == 1 && self.hours.len() == 1 && self.weekdays.len() == 7 {
            return format!(
                "daily at {}:{:02}",
                self.hours.iter().next().unwrap_or(&0),
                self.minutes.iter().next().unwrap_or(&0)
            );
        }
        if self.minutes.len() == 1 && self.hours.len() == 1 {
            return format!(
                "at {}:{:02}",
                self.hours.iter().next().unwrap_or(&0),
                self.minutes.iter().next().unwrap_or(&0)
            );
        }
        if self.minutes.len() == 60 {
            return "every hour".into();
        }
        if self.minutes.contains(&0) && self.minutes.len() == 12 {
            return "every six hours".into();
        }
        format!(
            "{} minute(s), {} hour(s), {} day(s), {} month(s)",
            self.minutes.len(),
            self.hours.len(),
            self.days.len(),
            self.months.len()
        )
    }
}

fn parse_field(field: &str, min: u32, max: u32, _allowed: &[&str]) -> Result<BTreeSet<u32>> {
    let mut values = BTreeSet::new();
    for part in field.split(',') {
        let part = part.trim();
        if part.is_empty() {
            bail!("the cron field `{field}` has an empty entry");
        }
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => {
                let step: u32 = step
                    .parse()
                    .map_err(|_| anyhow::anyhow!("`{step}` is not a valid step"))?;
                if step == 0 {
                    bail!("the step must be greater than zero");
                }
                (range, step)
            }
            None => (part, 1),
        };
        let (start, end) = if range == "*" {
            (min, max)
        } else if let Some((from, to)) = range.split_once('-') {
            let from: u32 = from
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("`{from}` is not a valid number"))?;
            let to: u32 = to
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("`{to}` is not a valid number"))?;
            (from, to)
        } else {
            let single: u32 = range
                .parse()
                .map_err(|_| anyhow::anyhow!("`{range}` is not a valid number"))?;
            (single, single)
        };
        if start < min || end > max || start > end {
            bail!("the value `{range}` is outside the allowed range {min}-{max}");
        }
        let mut value = start;
        while value <= end {
            values.insert(value);
            value += step;
        }
    }
    if values.is_empty() {
        bail!("the cron field `{field}` does not select anything");
    }
    Ok(values)
}

fn parse_weekday(field: &str) -> Result<BTreeSet<u32>> {
    let names = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
    let mut values = BTreeSet::new();
    for part in field.split(',') {
        let part = part.trim();
        if part == "*" {
            return Ok((0..=6).collect());
        }
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => {
                let step: u32 = step
                    .parse()
                    .map_err(|_| anyhow::anyhow!("`{step}` is not a valid step"))?;
                if step == 0 {
                    bail!("the step must be greater than zero");
                }
                (range, step)
            }
            None => (part, 1),
        };
        let (start, end) = if let Some((from, to)) = range.split_once('-') {
            (weekday_number(from, &names)?, weekday_number(to, &names)?)
        } else {
            let single = weekday_number(range, &names)?;
            (single, single)
        };
        let mut value = start;
        while value <= end {
            values.insert(value);
            value += step;
            if value > 6 {
                break;
            }
        }
    }
    if values.is_empty() {
        bail!("the weekday field does not select anything");
    }
    Ok(values)
}

fn weekday_number(value: &str, names: &[&str; 7]) -> Result<u32> {
    let value = value.trim().to_ascii_lowercase();
    if let Ok(number) = value.parse::<u32>() {
        if number <= 7 {
            // Both 0 and 7 mean Sunday.
            return Ok(number % 7);
        }
        bail!("`{value}` is not a valid weekday");
    }
    for (index, name) in names.iter().enumerate() {
        if *name == value {
            return Ok(index as u32);
        }
    }
    bail!("`{value}` is not a valid weekday");
}

/// A cron job the panel manages.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CronJob {
    pub id: String,
    pub site_id: Option<String>,
    pub user: String,
    pub expression: String,
    pub command: String,
    pub last_run_at: Option<String>,
    pub last_status: Option<String>,
    pub created_at: String,
}

impl CronJob {
    /// The crontab line for this job.
    pub fn crontab_line(&self) -> String {
        format!("{} {} {}", self.expression, self.user, self.command)
    }
}

/// A recorded run of a cron job.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct RunRecord {
    pub id: String,
    pub job_id: String,
    pub started_at: String,
    pub duration_ms: i64,
    pub exit_code: i64,
    pub output: String,
}

/// Validate a cron expression and describe it.
pub fn describe(expression: &str) -> Result<ScheduleDescription> {
    let schedule = Schedule::parse(expression)?;
    let next = schedule.next_run(Utc::now());
    Ok(ScheduleDescription {
        expression: expression.to_string(),
        summary: schedule.summary(),
        next_run: next.map(|value| value.to_rfc3339()),
    })
}

/// The last moment of a day, used by the scheduler.
pub fn end_of_day(day: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&day.and_time(NaiveTime::from_hms_opt(23, 59, 59).unwrap_or_default()))
}

/// The directory where the generated crontab fragment lives.
pub fn crontab_dir() -> PathBuf {
    std::env::var_os("SYNTHPANEL_CRON_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/etc/cron.d"))
}

/// Render the crontab fragment for a set of jobs.
pub fn render_crontab(jobs: &[CronJob]) -> String {
    let mut out = String::from(
        "# Generated by SynthPanel — do not edit; changes are overwritten.\n\
         SHELL=/bin/sh\n\
         PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin\n\n",
    );
    for job in jobs {
        out.push_str(&format!(
            "{} {} {}\n",
            job.expression.trim(),
            job.user,
            job.command.trim()
        ));
    }
    out
}

/// Write the crontab fragment for every managed job.
pub async fn sync_crontab(pool: &SqlitePool, dir: &Path) -> Result<PathBuf> {
    let jobs = list_jobs(pool).await?;
    let contents = render_crontab(&jobs);
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let path = dir.join("synthpanel");
    std::fs::write(&path, contents).with_context(|| format!("cannot write {}", path.display()))?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))?;
    Ok(path)
}

/// List the managed cron jobs.
pub async fn list_jobs(pool: &SqlitePool) -> Result<Vec<CronJob>> {
    let jobs = sqlx::query_as::<_, CronJob>(
        "SELECT id, site_id, user, expression, command, last_run_at, last_status, created_at
         FROM cron_jobs ORDER BY created_at",
    )
    .fetch_all(pool)
    .await?;
    Ok(jobs)
}

/// The recorded runs of a job, newest first.
pub async fn list_runs(pool: &SqlitePool, job_id: &str, limit: i64) -> Result<Vec<RunRecord>> {
    let runs = sqlx::query_as::<_, RunRecord>(
        "SELECT id, job_id, started_at, duration_ms, exit_code, output
         FROM cron_runs WHERE job_id = ? ORDER BY started_at DESC LIMIT ?",
    )
    .bind(job_id)
    .bind(limit.clamp(1, 500))
    .fetch_all(pool)
    .await?;
    Ok(runs)
}

/// Execute a job now and record the run.
pub async fn run_job(pool: &SqlitePool, job: &CronJob) -> Result<RunRecord> {
    let started_at = Utc::now().to_rfc3339();
    let started = std::time::Instant::now();

    let mut command = if is_root() && job.user != "root" {
        let mut command = tokio::process::Command::new("runuser");
        command
            .arg("-u")
            .arg(&job.user)
            .arg("--")
            .arg("sh")
            .arg("-c");
        command
    } else {
        let mut command = tokio::process::Command::new("sh");
        command.arg("-c");
        command
    };
    command.arg(&job.command);
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    command.stdin(std::process::Stdio::null());

    let output = command.output().await;
    let duration_ms = started.elapsed().as_millis() as i64;
    let (exit_code, text) = match output {
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.is_empty() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&stderr);
            }
            // Keep the output bounded; the UI only shows a tail.
            if text.len() > 64 * 1024 {
                text = text[text.len() - 64 * 1024..].to_string();
            }
            (output.status.code().unwrap_or(-1) as i64, text)
        }
        Err(error) => (-1, format!("cannot start the command: {error}")),
    };

    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO cron_runs (id, job_id, started_at, duration_ms, exit_code, output)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&job.id)
    .bind(&started_at)
    .bind(duration_ms)
    .bind(exit_code)
    .bind(&text)
    .execute(pool)
    .await?;

    sqlx::query("UPDATE cron_jobs SET last_run_at = ?, last_status = ? WHERE id = ?")
        .bind(&started_at)
        .bind(if exit_code == 0 { "ok" } else { "failed" })
        .bind(&job.id)
        .execute(pool)
        .await?;

    Ok(RunRecord {
        id,
        job_id: job.id.clone(),
        started_at,
        duration_ms,
        exit_code,
        output: text,
    })
}

/// Run every job that is due in the current minute.
pub async fn run_due(pool: &SqlitePool, dir: &Path) -> Result<usize> {
    let now = Utc::now();
    let jobs = list_jobs(pool).await?;
    let mut ran = 0;
    for job in jobs {
        let schedule = match Schedule::parse(&job.expression) {
            Ok(schedule) => schedule,
            Err(_) => continue,
        };
        if !schedule.matches(now) {
            continue;
        }
        // Do not run twice in the same minute (the scheduler ticks every 60s).
        if let Some(last) = &job.last_run_at {
            if let Ok(last) = DateTime::parse_from_rfc3339(last) {
                if last.with_timezone(&Utc).format("%Y%m%d%H%M").to_string()
                    == now.format("%Y%m%d%H%M").to_string()
                {
                    continue;
                }
            }
        }
        match run_job(pool, &job).await {
            Ok(record) => {
                ran += 1;
                if record.exit_code != 0 {
                    tracing::warn!(job = %job.id, exit = record.exit_code, "cron job failed");
                    let _ = crate::alerts::emit(
                        pool,
                        crate::alerts::events::CRON_FAILED,
                        "warning",
                        &format!(
                            "the cron job `{}` exited with {}",
                            job.command, record.exit_code
                        ),
                        &json!({ "job_id": job.id, "command": job.command }),
                    )
                    .await;
                }
            }
            Err(error) => tracing::warn!(job = %job.id, %error, "cron job could not run"),
        }
    }
    if ran > 0 {
        if let Err(error) = sync_crontab(pool, dir).await {
            tracing::warn!(%error, "cannot refresh the crontab");
        }
    }
    Ok(ran)
}

fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_simple_expression() {
        let schedule = Schedule::parse("0 3 * * *").unwrap();
        assert_eq!(schedule.minutes.len(), 1);
        assert_eq!(schedule.hours.iter().next(), Some(&3));
        assert_eq!(schedule.days.len(), 31);
        assert_eq!(schedule.months.len(), 12);
        assert_eq!(schedule.weekdays.len(), 7);
    }

    #[test]
    fn rejects_malformed_expressions() {
        assert!(Schedule::parse("0 3 * *").is_err());
        assert!(Schedule::parse("0 3 * * * *").is_err());
        assert!(Schedule::parse("99 3 * * *").is_err());
        assert!(Schedule::parse("0 25 * * *").is_err());
        assert!(Schedule::parse("0 3 * * nope").is_err());
        assert!(Schedule::parse("0 3 * * */0").is_err());
    }

    #[test]
    fn handles_ranges_and_steps() {
        let schedule = Schedule::parse("*/15 9-17 * * 1-5").unwrap();
        assert_eq!(schedule.minutes.len(), 4);
        assert_eq!(schedule.hours.len(), 9);
        assert_eq!(schedule.weekdays.len(), 5);
    }

    #[test]
    fn computes_the_next_run() {
        let schedule = Schedule::parse("0 3 * * *").unwrap();
        let after = Utc.with_ymd_and_hms(2026, 1, 2, 2, 0, 0).unwrap();
        let next = schedule.next_run(after).unwrap();
        assert_eq!(next.to_rfc3339(), "2026-01-02T03:00:00+00:00");

        // After the run time it moves to the next day.
        let after = Utc.with_ymd_and_hms(2026, 1, 2, 3, 30, 0).unwrap();
        let next = schedule.next_run(after).unwrap();
        assert_eq!(next.to_rfc3339(), "2026-01-03T03:00:00+00:00");
    }

    #[test]
    fn next_run_handles_weekday_restrictions() {
        // Every Monday at 04:00.
        let schedule = Schedule::parse("0 4 * * 1").unwrap();
        let after = Utc.with_ymd_and_hms(2026, 1, 2, 0, 0, 0).unwrap(); // a Friday
        let next = schedule.next_run(after).unwrap();
        assert_eq!(next.to_rfc3339(), "2026-01-05T04:00:00+00:00");
    }

    #[test]
    fn matches_exact_minutes() {
        let schedule = Schedule::parse("30 2 * * *").unwrap();
        assert!(schedule.matches(Utc.with_ymd_and_hms(2026, 5, 6, 2, 30, 0).unwrap()));
        assert!(!schedule.matches(Utc.with_ymd_and_hms(2026, 5, 6, 2, 31, 0).unwrap()));
        assert!(!schedule.matches(Utc.with_ymd_and_hms(2026, 5, 6, 3, 30, 0).unwrap()));
    }

    #[test]
    fn describes_a_schedule() {
        let description = describe("0 3 * * *").unwrap();
        assert!(description.summary.contains("daily"));
        assert!(description.next_run.is_some());
    }

    #[test]
    fn rejects_a_bad_expression_in_describe() {
        assert!(describe("nonsense").is_err());
    }

    #[test]
    fn weekday_names_are_understood() {
        let schedule = Schedule::parse("0 0 * * sun").unwrap();
        assert!(schedule.weekdays.contains(&0));
        let schedule = Schedule::parse("0 0 * * 7").unwrap();
        assert!(schedule.weekdays.contains(&0));
    }
}
