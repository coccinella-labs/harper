// Copyright 2026 coccinella-labs
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Session management service
//!
//! This module provides functionality for managing chat sessions,
//! including listing, viewing, and exporting sessions.

use crate::core::agents::ResolvedAgents;
use crate::core::error::{HarperError, HarperResult};
use crate::core::io_traits::{Input, Output};
use crate::core::plan::PlanState;
use crate::core::Message;
use crate::memory::cache::CacheAlignedBuffer;
use crate::memory::storage::{
    load_active_agents, load_command_logs_for_session, load_history, load_latest_command_log,
    load_plan_state,
};
use chrono::Local;
use colored::*;
use rusqlite::Connection;
use serde_json;
use std::io::Write;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub user_id: Option<String>,
    pub title: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default)]
pub struct GlobalStats {
    pub total_sessions: usize,
    pub total_messages: usize,
    pub total_commands: usize,
    pub approved_commands: usize,
    pub avg_command_duration_ms: f64,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SessionStateView {
    pub session_id: String,
    pub user_id: Option<String>,
    pub messages: Vec<Message>,
    pub plan: Option<PlanState>,
    pub agents: Option<ResolvedAgents>,
    pub agents_rendered: Option<String>,
    pub agents_effective_rendered: Option<String>,
}

/// Service for managing chat sessions
pub struct SessionService<'a> {
    conn: &'a Connection,
    input: Arc<dyn Input>,
    output: Arc<dyn Output>,
}

impl<'a> SessionService<'a> {
    const AUDIT_SUMMARY_LIMIT: usize = 5;
    const AUDIT_EXPORT_LIMIT: usize = 10;
    /// Create a new session service
    pub fn new(conn: &'a Connection) -> Self {
        Self::with_io(
            conn,
            crate::core::io_traits::StdInput,
            crate::core::io_traits::StdOutput,
        )
    }

    /// Create a new session service with custom I/O (for testing)
    pub fn with_io<I, O>(conn: &'a Connection, input: I, output: O) -> Self
    where
        I: Input + 'static,
        O: Output + 'static,
    {
        Self {
            conn,
            input: Arc::new(input),
            output: Arc::new(output),
        }
    }

    /// Get global usage statistics
    pub fn get_global_stats(&self) -> HarperResult<GlobalStats> {
        let total_sessions: usize =
            self.conn
                .query_row("SELECT COUNT(*) FROM sessions", [], |r| {
                    r.get::<_, i64>(0).map(|count| count as usize)
                })?;
        let total_messages: usize =
            self.conn
                .query_row("SELECT COUNT(*) FROM messages", [], |r| {
                    r.get::<_, i64>(0).map(|count| count as usize)
                })?;
        let total_commands: usize =
            self.conn
                .query_row("SELECT COUNT(*) FROM command_logs", [], |r| {
                    r.get::<_, i64>(0).map(|count| count as usize)
                })?;
        let approved_commands: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM command_logs WHERE approved = 1",
            [],
            |r| r.get::<_, i64>(0).map(|count| count as usize),
        )?;
        let avg_duration: f64 = self
            .conn
            .query_row(
                "SELECT AVG(duration_ms) FROM command_logs WHERE duration_ms IS NOT NULL",
                [],
                |r| r.get::<_, Option<f64>>(0),
            )?
            .unwrap_or(0.0);

        Ok(GlobalStats {
            total_sessions,
            total_messages,
            total_commands,
            approved_commands,
            avg_command_duration_ms: avg_duration,
        })
    }

    pub fn get_global_stats_for_user(&self, user_id: &str) -> HarperResult<GlobalStats> {
        let total_sessions: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM sessions WHERE user_id = ?1",
            [user_id],
            |r| r.get::<_, i64>(0).map(|count| count as usize),
        )?;
        let total_messages: usize = self.conn.query_row(
            "SELECT COUNT(*)
             FROM messages m
             JOIN sessions s ON s.id = m.session_id
             WHERE s.user_id = ?1",
            [user_id],
            |r| r.get::<_, i64>(0).map(|count| count as usize),
        )?;
        let total_commands: usize = self.conn.query_row(
            "SELECT COUNT(*)
             FROM command_logs c
             JOIN sessions s ON s.id = c.session_id
             WHERE s.user_id = ?1",
            [user_id],
            |r| r.get::<_, i64>(0).map(|count| count as usize),
        )?;
        let approved_commands: usize = self.conn.query_row(
            "SELECT COUNT(*)
             FROM command_logs c
             JOIN sessions s ON s.id = c.session_id
             WHERE s.user_id = ?1 AND c.approved = 1",
            [user_id],
            |r| r.get::<_, i64>(0).map(|count| count as usize),
        )?;
        let avg_duration: f64 = self
            .conn
            .query_row(
                "SELECT AVG(c.duration_ms)
                 FROM command_logs c
                 JOIN sessions s ON s.id = c.session_id
                 WHERE s.user_id = ?1 AND c.duration_ms IS NOT NULL",
                [user_id],
                |r| r.get::<_, Option<f64>>(0),
            )?
            .unwrap_or(0.0);

        Ok(GlobalStats {
            total_sessions,
            total_messages,
            total_commands,
            approved_commands,
            avg_command_duration_ms: avg_duration,
        })
    }

    /// List all previous sessions (returns data)
    pub fn list_sessions_data(&self) -> HarperResult<Vec<Session>> {
        self.list_sessions_data_inner(None)
    }

    pub fn list_sessions_data_for_user(&self, user_id: &str) -> HarperResult<Vec<Session>> {
        self.list_sessions_data_inner(Some(user_id))
    }

    /// List all previous sessions
    pub fn list_sessions(&self) -> HarperResult<()> {
        let sessions = self.list_sessions_data()?;
        self.output.println(&"Previous Sessions:".bold().yellow())?;
        if sessions.is_empty() {
            self.output
                .println("No previous sessions found. Start a chat to create one.")?;
            return Ok(());
        }

        for (i, session) in sessions.iter().enumerate() {
            let summary_text = match load_latest_command_log(self.conn, &session.id) {
                Ok(Some(entry)) => {
                    let approval_state = if entry.requires_approval {
                        if entry.approved {
                            "approved"
                        } else {
                            "rejected"
                        }
                    } else {
                        "auto"
                    };
                    let exit = entry
                        .exit_code
                        .map(|code| format!("exit {}", code))
                        .unwrap_or_else(|| "no exit".to_string());
                    format!("cmd: {} [{} | {}]", entry.status, approval_state, exit)
                }
                Ok(None) => "cmd: none".to_string(),
                Err(err) => {
                    eprintln!(
                        "Warning: failed to load audit summary for {}: {}",
                        session.id, err
                    );
                    "cmd: ?".to_string()
                }
            };

            self.output.println(&format!(
                "{}: {} ({}) - {}",
                i + 1,
                session.id,
                session.created_at,
                summary_text
            ))?;
        }
        Ok(())
    }

    /// View a specific session's history (returns data)
    pub fn view_session_data(&self, session_id: &str) -> HarperResult<Vec<crate::core::Message>> {
        let history = load_history(self.conn, session_id)?;
        Ok(history)
    }

    pub fn view_session_plan_data(&self, session_id: &str) -> HarperResult<Option<PlanState>> {
        load_plan_state(self.conn, session_id)
    }

    pub fn view_session_agents_data(
        &self,
        session_id: &str,
    ) -> HarperResult<Option<ResolvedAgents>> {
        load_active_agents(self.conn, session_id)
    }

    pub fn load_session_state_view(&self, session_id: &str) -> HarperResult<SessionStateView> {
        self.load_session_state_view_inner(session_id, None)
    }

    pub fn load_session_state_view_for_user(
        &self,
        session_id: &str,
        user_id: &str,
    ) -> HarperResult<Option<SessionStateView>> {
        self.load_session_state_view_inner(session_id, Some(user_id))
            .map(Some)
            .or_else(|err| match err {
                HarperError::Validation(message) if message == "session not found" => Ok(None),
                other => Err(other),
            })
    }

    pub fn delete_session_for_user(&self, session_id: &str, user_id: &str) -> HarperResult<bool> {
        let deleted = self.conn.execute(
            "DELETE FROM sessions WHERE id = ?1 AND user_id = ?2",
            [session_id, user_id],
        )?;
        Ok(deleted > 0)
    }

    pub fn delete_session(&self, session_id: &str) -> HarperResult<bool> {
        let deleted = self
            .conn
            .execute("DELETE FROM sessions WHERE id = ?1", [session_id])?;
        Ok(deleted > 0)
    }

    fn list_sessions_data_inner(&self, user_id: Option<&str>) -> HarperResult<Vec<Session>> {
        let mut stmt = if user_id.is_some() {
            self.conn.prepare(
                "SELECT id, user_id, title, created_at, updated_at
                 FROM sessions
                 WHERE user_id = ?1
                 ORDER BY updated_at DESC, created_at DESC",
            )?
        } else {
            self.conn.prepare(
                "SELECT id, user_id, title, created_at, updated_at
                 FROM sessions
                 ORDER BY updated_at DESC, created_at DESC",
            )?
        };
        let rows = if let Some(user_id) = user_id {
            stmt.query_map([user_id], Self::map_session_row)?
        } else {
            stmt.query_map([], Self::map_session_row)?
        };
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    fn map_session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Session> {
        Ok(Session {
            id: row.get(0)?,
            user_id: row.get(1)?,
            title: row.get(2)?,
            created_at: row.get(3)?,
            updated_at: row.get(4)?,
        })
    }

    fn load_session_state_view_inner(
        &self,
        session_id: &str,
        user_id: Option<&str>,
    ) -> HarperResult<SessionStateView> {
        let session = self.load_session_metadata(session_id, user_id)?;
        let messages = load_history(self.conn, session_id)?;
        let plan = load_plan_state(self.conn, session_id)?;
        let agents = load_active_agents(self.conn, session_id)?;
        let agents_rendered = agents
            .as_ref()
            .and_then(|resolved| resolved.render_for_prompt());
        let agents_effective_rendered = agents
            .as_ref()
            .and_then(|resolved| resolved.render_effective_for_display());

        Ok(SessionStateView {
            session_id: session_id.to_string(),
            user_id: session.user_id,
            messages,
            plan,
            agents,
            agents_rendered,
            agents_effective_rendered,
        })
    }

    fn load_session_metadata(
        &self,
        session_id: &str,
        user_id: Option<&str>,
    ) -> HarperResult<Session> {
        let mut stmt = if user_id.is_some() {
            self.conn.prepare(
                "SELECT id, user_id, title, created_at, updated_at
                 FROM sessions
                 WHERE id = ?1 AND user_id = ?2",
            )?
        } else {
            self.conn.prepare(
                "SELECT id, user_id, title, created_at, updated_at
                 FROM sessions
                 WHERE id = ?1",
            )?
        };

        let session = if let Some(user_id) = user_id {
            stmt.query_row([session_id, user_id], Self::map_session_row)
        } else {
            stmt.query_row([session_id], Self::map_session_row)
        };

        session.map_err(|err| match err {
            rusqlite::Error::QueryReturnedNoRows => {
                HarperError::Validation("session not found".to_string())
            }
            other => HarperError::Database(other.to_string()),
        })
    }

    /// View a specific session's history
    pub fn view_session(&self) -> HarperResult<()> {
        self.output.print("Enter session ID to view: ")?;
        self.output.flush()?;
        let session_id = self.input.read_line()?.trim().to_string();

        let history = self.view_session_data(&session_id)?;
        let total_messages = history.len();

        // Show only the last 20 messages to prevent overwhelming output
        const MAX_DISPLAY: usize = 20;
        let display_start = total_messages.saturating_sub(MAX_DISPLAY);

        self.output.println(&format!(
            "
{} (showing last {} of {} messages)
",
            "Session History:".bold().yellow(),
            total_messages.saturating_sub(display_start),
            total_messages
        ))?;

        for msg in &history[display_start..] {
            let line = match msg.role.as_str() {
                "user" => format!("{} {}", "You:".bold().blue(), msg.content.blue()),
                "assistant" => format!("{} {}", "Assistant:".bold().green(), msg.content.green()),
                "system" => format!("{} {}", "System:".bold().magenta(), msg.content.magenta()),
                _ => format!("{}: {}", msg.role, msg.content),
            };
            self.output.println(&line)?;
        }

        if total_messages > MAX_DISPLAY {
            self.output.println(&format!(
                "
{} Use export to view the full transcript.",
                "Note:".bold().cyan()
            ))?;
        }

        self.print_plan_summary(&session_id)?;
        self.print_agents_summary(&session_id)?;
        self.print_audit_summary(&session_id, Self::AUDIT_SUMMARY_LIMIT)?;

        Ok(())
    }

    /// Export a session's history to a file
    pub fn export_session(&self) -> HarperResult<()> {
        self.output.print("Enter session ID to export: ")?;
        self.output.flush()?;
        let session_id = self.input.read_line()?.trim().to_string();

        self.output.print("Export format (txt/json) [txt]: ")?;
        self.output.flush()?;
        let format_choice = self.input.read_line()?.trim().to_lowercase();
        let is_json = format_choice == "json";

        let history = load_history(self.conn, &session_id)?;

        if history.is_empty() {
            self.output
                .println(&format!("No history found for session {}", session_id))?;
            return Ok(());
        }

        let default_filename = format!("harper_export_{}", session_id);
        let extension = if is_json { "json" } else { "txt" };
        let default_path = format!("{}.{}", default_filename, extension);

        self.output
            .print(&format!("Enter output file path [{}]: ", default_path))?;
        self.output.flush()?;
        let output_path = self.input.read_line()?.trim().to_string();

        let output_path = if output_path.is_empty() {
            default_path
        } else {
            output_path
        };

        if is_json {
            let plan = load_plan_state(self.conn, &session_id)?;
            let json = serde_json::json!({
                "messages": history,
                "plan": plan,
            });
            let json = serde_json::to_string_pretty(&json)?;
            std::fs::write(&output_path, json)?;
        } else {
            let mut buffer = CacheAlignedBuffer::with_capacity(history.len() * 128);
            self.write_transcript(&mut buffer, &history)?;
            self.write_plan_section(&mut buffer, &session_id)?;
            self.write_audit_section(&mut buffer, &session_id)?;
            std::fs::write(&output_path, buffer.as_slice())?;
        }

        self.output.println(&format!(
            "Successfully exported {} messages to {}",
            history.len(),
            output_path
        ))?;

        Ok(())
    }

    pub fn export_session_by_id(&self, session_id: &str) -> HarperResult<String> {
        let history = load_history(self.conn, session_id)?;

        if history.is_empty() {
            return Err(HarperError::File(format!(
                "No history found for session {}",
                session_id
            )));
        }

        let default_filename = format!("harper_export_{}", session_id);
        let output_path = format!("{}.txt", default_filename);

        let mut buffer = CacheAlignedBuffer::with_capacity(history.len() * 128);
        self.write_transcript(&mut buffer, &history)?;
        self.write_plan_section(&mut buffer, session_id)?;
        self.write_audit_section(&mut buffer, session_id)?;
        std::fs::write(&output_path, buffer.as_slice())?;

        Ok(output_path)
    }

    pub fn export_session_by_id_for_user(
        &self,
        session_id: &str,
        user_id: &str,
    ) -> HarperResult<String> {
        self.load_session_state_view_for_user(session_id, user_id)?
            .ok_or_else(|| HarperError::Validation("session not found".to_string()))?;
        self.export_session_by_id(session_id)
    }

    fn write_transcript<W: Write>(&self, writer: &mut W, history: &[Message]) -> HarperResult<()> {
        for msg in history {
            writeln!(
                writer,
                "[{}] {}: {}",
                Local::now().format("%Y-%m-%d %H:%M:%S"),
                msg.role,
                msg.content.replace('\n', "\n  ")
            )?;
        }
        Ok(())
    }

    fn print_audit_summary(&self, session_id: &str, limit: usize) -> HarperResult<()> {
        let entries = load_command_logs_for_session(self.conn, session_id, limit)?;
        self.output.println(&format!(
            "
{} (last {} commands)",
            "Command Audit Summary:".bold().yellow(),
            limit
        ))?;

        if entries.is_empty() {
            self.output
                .println("No commands have been recorded for this session.")?;
            return Ok(());
        }

        for entry in entries {
            let approval_state = if entry.requires_approval {
                if entry.approved {
                    "approved"
                } else {
                    "rejected"
                }
            } else {
                "auto"
            };
            let exit = entry
                .exit_code
                .map(|code| format!("exit {}", code))
                .unwrap_or_else(|| "no exit code".to_string());
            let duration = entry
                .duration_ms
                .map(|ms| format!("{} ms", ms))
                .unwrap_or_else(|| "-".to_string());
            self.output.println(&format!(
                "- {} [{} | {} | {}]",
                entry.command, entry.status, approval_state, exit
            ))?;
            self.output.println(&format!("  Duration: {}", duration))?;
        }
        Ok(())
    }

    fn print_plan_summary(&self, session_id: &str) -> HarperResult<()> {
        if let Some(plan) = load_plan_state(self.conn, session_id)? {
            self.output
                .println(&format!("\n{}", "Plan:".bold().yellow()))?;
            if let Some(explanation) = plan.explanation {
                self.output.println(&format!("  {}", explanation))?;
            }
            for item in plan.items {
                self.output.println(&format!(
                    "  - [{}] {}",
                    match item.status {
                        crate::core::plan::PlanStepStatus::Pending => "pending",
                        crate::core::plan::PlanStepStatus::InProgress => "in_progress",
                        crate::core::plan::PlanStepStatus::Completed => "completed",
                        crate::core::plan::PlanStepStatus::Blocked => "blocked",
                    },
                    item.step
                ))?;
            }
        }
        Ok(())
    }

    fn write_audit_section<W: Write>(&self, writer: &mut W, session_id: &str) -> HarperResult<()> {
        let entries =
            load_command_logs_for_session(self.conn, session_id, Self::AUDIT_EXPORT_LIMIT)?;
        writeln!(
            writer,
            "\n---\nCommand Audit (last {} commands):",
            Self::AUDIT_EXPORT_LIMIT
        )?;
        if entries.is_empty() {
            writeln!(writer, "No commands were recorded for this session.")?;
            return Ok(());
        }

        for entry in entries {
            let approval_state = if entry.requires_approval {
                if entry.approved {
                    "approved"
                } else {
                    "rejected"
                }
            } else {
                "auto"
            };
            let exit = entry
                .exit_code
                .map(|code| format!("exit {}", code))
                .unwrap_or_else(|| "no exit code".to_string());
            let duration = entry
                .duration_ms
                .map(|ms| format!("{} ms", ms))
                .unwrap_or_else(|| "-".to_string());
            writeln!(
                writer,
                "[{}] {} [{} | {} | {}] {}",
                Local::now().format("%Y-%m-%d %H:%M:%S"),
                entry.command,
                entry.status,
                approval_state,
                exit,
                duration
            )?;
        }
        Ok(())
    }

    fn write_plan_section<W: Write>(&self, writer: &mut W, session_id: &str) -> HarperResult<()> {
        if let Some(plan) = load_plan_state(self.conn, session_id)? {
            writeln!(writer, "\n---\nPlan:")?;
            if let Some(explanation) = plan.explanation {
                writeln!(writer, "{}", explanation)?;
            }
            for item in plan.items {
                let status = match item.status {
                    crate::core::plan::PlanStepStatus::Pending => "pending",
                    crate::core::plan::PlanStepStatus::InProgress => "in_progress",
                    crate::core::plan::PlanStepStatus::Completed => "completed",
                    crate::core::plan::PlanStepStatus::Blocked => "blocked",
                };
                writeln!(writer, "- [{}] {}", status, item.step)?;
            }
        }
        Ok(())
    }

    fn print_agents_summary(&self, session_id: &str) -> HarperResult<()> {
        if let Some(agents) = load_active_agents(self.conn, session_id)? {
            if !agents.sources.is_empty() {
                self.output.println(&format!(
                    "\n{}",
                    "Active AGENTS.md Sources:".bold().yellow()
                ))?;
                for source in agents.sources {
                    self.output
                        .println(&format!("  - {}", source.path.display()))?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SessionService;
    use crate::memory::storage::{
        init_db, insert_command_log, save_message, save_session_for_user, CommandLogRecord,
    };
    use rusqlite::Connection;

    #[test]
    fn user_scoped_stats_only_count_owned_sessions() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        init_db(&conn).expect("init db");

        save_session_for_user(&conn, "session-a", "user-a").expect("save session a");
        save_session_for_user(&conn, "session-b", "user-b").expect("save session b");

        save_message(&conn, "session-a", "user", "hello from a").expect("message a");
        save_message(&conn, "session-b", "user", "hello from b").expect("message b");

        insert_command_log(
            &conn,
            &CommandLogRecord::new(
                Some("session-a"),
                "cargo test",
                "tui",
                false,
                true,
                "completed",
                Some(0),
                Some(25),
                None,
                None,
                None,
            ),
        )
        .expect("command a");
        insert_command_log(
            &conn,
            &CommandLogRecord::new(
                Some("session-b"),
                "cargo check",
                "tui",
                false,
                false,
                "failed",
                Some(1),
                Some(50),
                None,
                None,
                Some("boom".to_string()),
            ),
        )
        .expect("command b");

        let service = SessionService::new(&conn);
        let stats = service
            .get_global_stats_for_user("user-a")
            .expect("stats for user a");

        assert_eq!(stats.total_sessions, 1);
        assert_eq!(stats.total_messages, 1);
        assert_eq!(stats.total_commands, 1);
        assert_eq!(stats.approved_commands, 1);
        assert_eq!(stats.avg_command_duration_ms, 25.0);
    }
}
