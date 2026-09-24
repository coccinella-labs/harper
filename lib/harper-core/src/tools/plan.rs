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

use crate::core::error::{HarperError, HarperResult};
use crate::core::plan::{
    AuthoringPlannedEdit, AuthoringValidationStep, PlanItem, PlanJobStatus, PlanLoopOutcome,
    PlanLoopStage, PlanRuntime, PlanState, PlanStepStatus, StructuredAuthoringPlan,
};
use rusqlite::Connection;

pub fn update_plan(
    conn: &Connection,
    session_id: &str,
    args: &serde_json::Value,
) -> HarperResult<String> {
    let items_value = args
        .get("items")
        .or_else(|| args.get("plan"))
        .and_then(|value| value.as_array())
        .ok_or_else(|| {
            HarperError::Validation("update_plan requires an items array".to_string())
        })?;

    if items_value.is_empty() {
        return Err(HarperError::Validation(
            "update_plan requires at least one plan item".to_string(),
        ));
    }

    let mut items = Vec::with_capacity(items_value.len());
    for raw_item in items_value {
        let step = raw_item
            .get("step")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                HarperError::Validation("each plan item requires a non-empty step".to_string())
            })?;
        let status = parse_status(raw_item.get("status"))?;
        items.push(PlanItem {
            step: step.to_string(),
            status,
            job_id: None,
        });
    }

    let explanation = args
        .get("explanation")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    let structured_authoring_plan = args
        .get("authoring_plan")
        .map(parse_authoring_plan)
        .transpose()?;

    let existing_runtime =
        crate::memory::storage::load_plan_state(conn, session_id)?.and_then(|plan| plan.runtime);
    let mut runtime = existing_runtime.unwrap_or_default();
    runtime.set_loop_stage(PlanLoopStage::Planning, Some("plan updated".to_string()));
    runtime.record_outcome(PlanLoopOutcome::Responded, Some("plan ready".to_string()));
    if let Some(authoring_plan) = structured_authoring_plan {
        runtime.set_authoring_structured_plan(authoring_plan);
    }

    let plan = PlanState {
        explanation: explanation.clone(),
        items,
        runtime: (!runtime.is_empty()).then_some(runtime),
        updated_at: None,
    };

    crate::memory::storage::save_plan_state(conn, session_id, &plan)?;

    let in_progress = plan
        .items
        .iter()
        .filter(|item| item.status == PlanStepStatus::InProgress)
        .count();
    let completed = plan
        .items
        .iter()
        .filter(|item| item.status == PlanStepStatus::Completed)
        .count();

    let mut summary = format!(
        "Plan updated: {} steps ({} completed, {} in progress).",
        plan.items.len(),
        completed,
        in_progress
    );
    if let Some(explanation) = explanation {
        summary.push_str(&format!(" {}", explanation));
    }

    Ok(summary)
}

pub fn set_plan_runtime(
    conn: &Connection,
    session_id: &str,
    runtime: Option<PlanRuntime>,
) -> HarperResult<()> {
    let Some(mut plan) = crate::memory::storage::load_plan_state(conn, session_id)? else {
        return Ok(());
    };
    plan.runtime = runtime;
    crate::memory::storage::save_plan_state(conn, session_id, &plan)
}

pub fn set_plan_runtime_state(
    conn: &Connection,
    session_id: &str,
    tool_name: &str,
    command: Option<String>,
    status: &str,
) -> HarperResult<()> {
    update_plan_runtime(conn, session_id, |runtime| {
        runtime.set_active_tool_state(tool_name.to_string(), command, status.to_string());
    })
}

pub fn set_plan_loop_stage(
    conn: &Connection,
    session_id: &str,
    stage: PlanLoopStage,
    feedback: Option<String>,
) -> HarperResult<()> {
    update_plan_runtime(conn, session_id, |runtime| {
        runtime.set_loop_stage(stage.clone(), feedback.clone());
    })
}

pub fn record_plan_loop_outcome(
    conn: &Connection,
    session_id: &str,
    outcome: PlanLoopOutcome,
    feedback: Option<String>,
) -> HarperResult<()> {
    update_plan_runtime(conn, session_id, |runtime| {
        runtime.record_outcome(outcome.clone(), feedback.clone());
    })
}

pub fn start_plan_job(
    conn: &Connection,
    session_id: &str,
    tool_name: &str,
    command: Option<String>,
    status: PlanJobStatus,
) -> HarperResult<()> {
    let Some(mut plan) = crate::memory::storage::load_plan_state(conn, session_id)? else {
        return Ok(());
    };
    let mut runtime = plan.runtime.unwrap_or_default();
    let job_id = runtime.start_job(tool_name.to_string(), command, status);
    if let Some(item) = plan
        .items
        .iter_mut()
        .find(|item| matches!(item.status, PlanStepStatus::InProgress))
    {
        item.job_id = Some(job_id);
    }
    plan.runtime = (!runtime.is_empty()).then_some(runtime);
    crate::memory::storage::save_plan_state(conn, session_id, &plan)
}

pub fn update_active_plan_job(
    conn: &Connection,
    session_id: &str,
    status: PlanJobStatus,
) -> HarperResult<()> {
    update_plan_runtime(conn, session_id, |runtime| {
        runtime.update_active_job_status(status);
    })
}

pub fn record_active_plan_retry_followup(
    conn: &Connection,
    session_id: &str,
    command: Option<String>,
) -> HarperResult<()> {
    let Some(mut plan) = crate::memory::storage::load_plan_state(conn, session_id)? else {
        return Ok(());
    };
    let Some(step) = plan
        .items
        .iter()
        .find(|item| matches!(item.status, PlanStepStatus::InProgress))
        .map(|item| item.step.clone())
    else {
        return Ok(());
    };
    let mut runtime = plan.runtime.unwrap_or_default();
    runtime.set_retry_or_replan_followup(step, command);
    plan.runtime = (!runtime.is_empty()).then_some(runtime);
    crate::memory::storage::save_plan_state(conn, session_id, &plan)
}

pub fn finish_active_plan_job(
    conn: &Connection,
    session_id: &str,
    status: PlanJobStatus,
) -> HarperResult<()> {
    finish_active_plan_job_with_output(conn, session_id, status, None, false)
}

pub fn finish_active_plan_job_with_output(
    conn: &Connection,
    session_id: &str,
    status: PlanJobStatus,
    output_preview: Option<String>,
    has_error_output: bool,
) -> HarperResult<()> {
    let Some(mut plan) = crate::memory::storage::load_plan_state(conn, session_id)? else {
        return Ok(());
    };
    let Some(mut runtime) = plan.runtime.take() else {
        return Ok(());
    };
    let active_job_id = runtime.active_job_id.clone();
    runtime.set_active_job_output(output_preview, has_error_output);
    runtime.finish_active_job(status.clone());

    if let Some(active_job_id) = active_job_id {
        if let Some(item_index) = plan
            .items
            .iter()
            .position(|item| item.job_id.as_deref() == Some(active_job_id.as_str()))
        {
            let step_text = plan.items[item_index].step.clone();
            let job_command = runtime
                .jobs
                .iter()
                .rev()
                .find(|job| job.job_id == active_job_id)
                .and_then(|job| job.command.clone())
                .or_else(|| {
                    runtime
                        .jobs
                        .iter()
                        .rev()
                        .find(|job| job.job_id == active_job_id)
                        .map(|job| job.tool.clone())
                });
            plan.items[item_index].job_id = None;
            match status {
                PlanJobStatus::Succeeded => {
                    plan.items[item_index].status = PlanStepStatus::Completed;
                    let next_step = if let Some(next_pending) = plan
                        .items
                        .iter_mut()
                        .find(|item| matches!(item.status, PlanStepStatus::Pending))
                    {
                        let next_step = next_pending.step.clone();
                        next_pending.status = PlanStepStatus::InProgress;
                        Some(next_step)
                    } else {
                        None
                    };
                    runtime.set_checkpoint_followup(step_text, next_step);
                }
                PlanJobStatus::Blocked | PlanJobStatus::Failed => {
                    plan.items[item_index].status = PlanStepStatus::Blocked;
                    runtime.set_retry_or_replan_followup(step_text, job_command);
                }
                PlanJobStatus::WaitingApproval | PlanJobStatus::Running => {}
            }
        }
    }

    plan.runtime = (!runtime.is_empty()).then_some(runtime);
    crate::memory::storage::save_plan_state(conn, session_id, &plan)
}

pub fn clear_active_plan_runtime(conn: &Connection, session_id: &str) -> HarperResult<()> {
    update_plan_runtime(conn, session_id, |runtime| {
        runtime.clear_active_state();
    })
}

pub fn append_active_plan_job_output(
    conn: &Connection,
    session_id: &str,
    chunk: &str,
    is_error: bool,
) -> HarperResult<()> {
    update_plan_runtime(conn, session_id, |runtime| {
        runtime.append_active_job_output(chunk, is_error);
    })
}

pub fn set_plan_step_status(
    conn: &Connection,
    session_id: &str,
    step_index: usize,
    status: PlanStepStatus,
) -> HarperResult<()> {
    let Some(mut plan) = crate::memory::storage::load_plan_state(conn, session_id)? else {
        return Ok(());
    };
    if step_index >= plan.items.len() {
        return Err(HarperError::Validation(format!(
            "plan step index {} is out of bounds",
            step_index
        )));
    }

    if matches!(status, PlanStepStatus::InProgress) {
        for (index, item) in plan.items.iter_mut().enumerate() {
            if index != step_index && matches!(item.status, PlanStepStatus::InProgress) {
                item.status = PlanStepStatus::Pending;
                item.job_id = None;
            }
        }
    }

    let item = &mut plan.items[step_index];
    item.status = status;
    item.job_id = None;

    crate::memory::storage::save_plan_state(conn, session_id, &plan)
}

pub fn clear_plan_state(conn: &Connection, session_id: &str) -> HarperResult<()> {
    crate::memory::storage::delete_plan_state(conn, session_id)
}

pub fn clear_plan_followup(conn: &Connection, session_id: &str) -> HarperResult<()> {
    let Some(mut plan) = crate::memory::storage::load_plan_state(conn, session_id)? else {
        return Ok(());
    };
    let Some(runtime) = plan.runtime.as_mut() else {
        return Ok(());
    };
    runtime.clear_followup();
    if runtime.is_empty() {
        plan.runtime = None;
    }
    crate::memory::storage::save_plan_state(conn, session_id, &plan)
}

pub fn seed_plan_authoring_context(
    conn: &Connection,
    session_id: &str,
    request: &str,
    candidate_files: Vec<String>,
) -> HarperResult<()> {
    update_plan_runtime(conn, session_id, |runtime| {
        runtime.seed_authoring_context(request.to_string(), candidate_files);
    })
}

pub fn mark_plan_authoring_plan_created(conn: &Connection, session_id: &str) -> HarperResult<()> {
    update_plan_runtime(conn, session_id, |runtime| {
        runtime.mark_authoring_plan_created();
    })
}

pub fn mark_plan_authoring_inspection(
    conn: &Connection,
    session_id: &str,
    inspected_files: Vec<String>,
) -> HarperResult<()> {
    update_plan_runtime(conn, session_id, |runtime| {
        runtime.mark_authoring_inspection(inspected_files);
    })
}

pub fn mark_plan_authoring_edit_applied(
    conn: &Connection,
    session_id: &str,
    edited_files: Vec<String>,
) -> HarperResult<()> {
    update_plan_runtime(conn, session_id, |runtime| {
        runtime.mark_authoring_edit_applied(edited_files);
    })
}
pub fn mark_plan_authoring_validated(conn: &Connection, session_id: &str) -> HarperResult<()> {
    update_plan_runtime(conn, session_id, |runtime| {
        runtime.mark_authoring_validated();
    })
}

pub fn replan_blocked_step(
    conn: &Connection,
    session_id: &str,
    step_index: usize,
) -> HarperResult<()> {
    let Some(mut plan) = crate::memory::storage::load_plan_state(conn, session_id)? else {
        return Ok(());
    };
    if step_index >= plan.items.len() {
        return Err(HarperError::Validation(format!(
            "plan step index {} is out of bounds",
            step_index
        )));
    }

    let original_step = plan.items[step_index].step.clone();
    for item in &mut plan.items {
        if matches!(item.status, PlanStepStatus::InProgress) {
            item.status = PlanStepStatus::Pending;
            item.job_id = None;
        }
    }

    plan.items[step_index] = PlanItem {
        step: format!("Revise approach for blocked step: {}", original_step),
        status: PlanStepStatus::InProgress,
        job_id: None,
    };
    plan.items.insert(
        step_index + 1,
        PlanItem {
            step: format!("Validate revised approach for: {}", original_step),
            status: PlanStepStatus::Pending,
            job_id: None,
        },
    );

    let note = format!("Planner replan generated after blocker: {}", original_step);
    plan.explanation = match plan.explanation.take() {
        Some(existing) if !existing.trim().is_empty() => Some(format!("{} {}", existing, note)),
        _ => Some(note),
    };
    if let Some(runtime) = plan.runtime.as_mut() {
        runtime.clear_followup();
        runtime.clear_active_state();
        if runtime.is_empty() {
            plan.runtime = None;
        }
    }

    crate::memory::storage::save_plan_state(conn, session_id, &plan)
}

fn update_plan_runtime<F>(conn: &Connection, session_id: &str, mutator: F) -> HarperResult<()>
where
    F: FnOnce(&mut PlanRuntime),
{
    let Some(mut plan) = crate::memory::storage::load_plan_state(conn, session_id)? else {
        return Ok(());
    };
    let mut runtime = plan.runtime.unwrap_or_default();
    mutator(&mut runtime);
    plan.runtime = (!runtime.is_empty()).then_some(runtime);
    crate::memory::storage::save_plan_state(conn, session_id, &plan)
}

fn parse_status(value: Option<&serde_json::Value>) -> HarperResult<PlanStepStatus> {
    let raw = value
        .and_then(|status| status.as_str())
        .unwrap_or("pending")
        .trim()
        .to_ascii_lowercase();

    match raw.as_str() {
        "pending" => Ok(PlanStepStatus::Pending),
        "in_progress" | "in-progress" | "in progress" => Ok(PlanStepStatus::InProgress),
        "completed" | "done" => Ok(PlanStepStatus::Completed),
        "blocked" => Ok(PlanStepStatus::Blocked),
        _ => Err(HarperError::Validation(format!(
            "invalid plan status '{}'",
            raw
        ))),
    }
}

fn parse_authoring_plan(value: &serde_json::Value) -> HarperResult<StructuredAuthoringPlan> {
    let object = value
        .as_object()
        .ok_or_else(|| HarperError::Validation("authoring_plan must be an object".to_string()))?;

    let parse_paths = |key: &str| -> HarperResult<Vec<String>> {
        let Some(raw) = object.get(key) else {
            return Ok(Vec::new());
        };
        let items = raw.as_array().ok_or_else(|| {
            HarperError::Validation(format!("authoring_plan.{} must be an array", key))
        })?;
        let mut paths = Vec::new();
        for item in items {
            let path = item
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    HarperError::Validation(format!(
                        "authoring_plan.{} entries must be non-empty strings",
                        key
                    ))
                })?;
            paths.push(path.to_string());
        }
        Ok(paths)
    };

    let parse_planned_edits = || -> HarperResult<Vec<AuthoringPlannedEdit>> {
        let Some(raw) = object.get("planned_edits") else {
            return Ok(Vec::new());
        };
        let items = raw.as_array().ok_or_else(|| {
            HarperError::Validation("authoring_plan.planned_edits must be an array".to_string())
        })?;
        let mut edits = Vec::new();
        for item in items {
            let edit = item.as_object().ok_or_else(|| {
                HarperError::Validation(
                    "authoring_plan.planned_edits entries must be objects".to_string(),
                )
            })?;
            let path = edit
                .get("path")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    HarperError::Validation(
                        "authoring_plan.planned_edits.path is required".to_string(),
                    )
                })?;
            let change = edit
                .get("change")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    HarperError::Validation(
                        "authoring_plan.planned_edits.change is required".to_string(),
                    )
                })?;
            let why = edit
                .get("why")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned);
            edits.push(AuthoringPlannedEdit {
                path: path.to_string(),
                change: change.to_string(),
                why,
            });
        }
        Ok(edits)
    };

    let parse_validation = || -> HarperResult<Vec<AuthoringValidationStep>> {
        let Some(raw) = object.get("validation_plan") else {
            return Ok(Vec::new());
        };
        let items = raw.as_array().ok_or_else(|| {
            HarperError::Validation("authoring_plan.validation_plan must be an array".to_string())
        })?;
        let mut steps = Vec::new();
        for item in items {
            let step = item.as_object().ok_or_else(|| {
                HarperError::Validation(
                    "authoring_plan.validation_plan entries must be objects".to_string(),
                )
            })?;
            let command = step
                .get("command")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    HarperError::Validation(
                        "authoring_plan.validation_plan.command is required".to_string(),
                    )
                })?;
            let scope = step
                .get("scope")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned);
            steps.push(AuthoringValidationStep {
                command: command.to_string(),
                scope,
            });
        }
        Ok(steps)
    };

    let plan = StructuredAuthoringPlan {
        primary_files: parse_paths("primary_files")?,
        supporting_files: parse_paths("supporting_files")?,
        validation_files: parse_paths("validation_files")?,
        planned_edits: parse_planned_edits()?,
        validation_plan: parse_validation()?,
    };

    if plan.primary_files.is_empty()
        && plan.supporting_files.is_empty()
        && plan.planned_edits.is_empty()
    {
        return Err(HarperError::Validation(
            "authoring_plan must include primary_files, supporting_files, or planned_edits"
                .to_string(),
        ));
    }

    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::{
        append_active_plan_job_output, clear_plan_followup, clear_plan_state,
        finish_active_plan_job, finish_active_plan_job_with_output, mark_plan_authoring_validated,
        replan_blocked_step, set_plan_step_status, start_plan_job, update_plan,
    };
    use crate::core::plan::{PlanItem, PlanJobStatus, PlanState, PlanStepStatus};
    use rusqlite::Connection;

    #[test]
    fn start_plan_job_links_current_in_progress_step() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::memory::storage::init_db(&conn).expect("init db");
        crate::memory::storage::save_plan_state(
            &conn,
            "plan-job-session",
            &PlanState {
                explanation: None,
                items: vec![PlanItem {
                    step: "Run migration".to_string(),
                    status: PlanStepStatus::InProgress,
                    job_id: None,
                }],
                runtime: None,
                updated_at: None,
            },
        )
        .expect("save plan");

        start_plan_job(
            &conn,
            "plan-job-session",
            "run_command",
            Some("cargo test".to_string()),
            PlanJobStatus::Running,
        )
        .expect("start job");

        let plan = crate::memory::storage::load_plan_state(&conn, "plan-job-session")
            .expect("load plan")
            .expect("plan present");
        assert_eq!(plan.runtime.as_ref().map(|rt| rt.jobs.len()), Some(1));
        assert_eq!(
            plan.items[0].job_id,
            plan.runtime
                .as_ref()
                .and_then(|runtime| runtime.active_job_id.clone())
        );
    }

    #[test]
    fn update_plan_persists_structured_authoring_plan() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::memory::storage::init_db(&conn).expect("init db");

        let args = serde_json::json!({
            "items": [{"step": "Inspect files", "status": "in_progress"}],
            "authoring_plan": {
                "primary_files": ["lib/harper-ui/src/interfaces/ui/widgets.rs"],
                "supporting_files": ["lib/harper-core/src/agent/chat.rs"],
                "planned_edits": [
                    {
                        "path": "lib/harper-ui/src/interfaces/ui/widgets.rs",
                        "change": "Adjust planner retry rendering",
                        "why": "UI render path"
                    }
                ],
                "validation_plan": [
                    {"command": "cargo check -p harper-ui", "scope": "narrow"}
                ]
            }
        });

        update_plan(&conn, "authoring-plan-session", &args).expect("update plan");

        let plan = crate::memory::storage::load_plan_state(&conn, "authoring-plan-session")
            .expect("load plan")
            .expect("plan present");
        let runtime = plan.runtime.expect("runtime present");
        let authoring = runtime.authoring.expect("authoring runtime present");
        let structured = authoring.structured_plan.expect("structured plan present");
        assert_eq!(
            structured.primary_files,
            vec!["lib/harper-ui/src/interfaces/ui/widgets.rs".to_string()]
        );
        assert_eq!(structured.validation_plan.len(), 1);
        assert!(
            authoring
                .edit_scope
                .iter()
                .any(|path| path == "lib/harper-ui/src/interfaces/ui/widgets.rs")
        );
    }

    #[test]
    fn mark_plan_authoring_validated_advances_phase() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::memory::storage::init_db(&conn).expect("init db");
        crate::memory::storage::save_plan_state(
            &conn,
            "authoring-validated-session",
            &PlanState {
                explanation: None,
                items: vec![PlanItem {
                    step: "Validate changes".to_string(),
                    status: PlanStepStatus::InProgress,
                    job_id: None,
                }],
                runtime: Some(crate::core::plan::PlanRuntime {
                    authoring: Some(crate::core::plan::AuthoringRuntime {
                        request: Some("refactor planner flow".to_string()),
                        phase: Some(crate::core::plan::AuthoringPhase::EditsApplied),
                        candidate_files: vec![],
                        inspected_files: vec![],
                        edit_scope: vec![],
                        structured_plan: None,
                    }),
                    ..Default::default()
                }),
                updated_at: None,
            },
        )
        .expect("save plan");

        mark_plan_authoring_validated(&conn, "authoring-validated-session")
            .expect("mark validated");

        let plan = crate::memory::storage::load_plan_state(&conn, "authoring-validated-session")
            .expect("load plan")
            .expect("plan present");
        assert_eq!(
            plan.runtime
                .and_then(|rt| rt.authoring)
                .and_then(|authoring| authoring.phase),
            Some(crate::core::plan::AuthoringPhase::Validated)
        );
    }

    #[test]
    fn finish_active_plan_job_advances_linked_step_on_success() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::memory::storage::init_db(&conn).expect("init db");
        crate::memory::storage::save_plan_state(
            &conn,
            "plan-finish-session",
            &PlanState {
                explanation: None,
                items: vec![
                    PlanItem {
                        step: "Run migration".to_string(),
                        status: PlanStepStatus::InProgress,
                        job_id: None,
                    },
                    PlanItem {
                        step: "Check output".to_string(),
                        status: PlanStepStatus::Pending,
                        job_id: None,
                    },
                ],
                runtime: None,
                updated_at: None,
            },
        )
        .expect("save plan");

        start_plan_job(
            &conn,
            "plan-finish-session",
            "run_command",
            Some("cargo test".to_string()),
            PlanJobStatus::Running,
        )
        .expect("start job");
        finish_active_plan_job(&conn, "plan-finish-session", PlanJobStatus::Succeeded)
            .expect("finish job");

        let plan = crate::memory::storage::load_plan_state(&conn, "plan-finish-session")
            .expect("load plan")
            .expect("plan present");
        assert_eq!(plan.items[0].status, PlanStepStatus::Completed);
        assert!(plan.items[0].job_id.is_none());
        assert_eq!(plan.items[1].status, PlanStepStatus::InProgress);
        assert_eq!(plan.runtime.as_ref().map(|rt| rt.jobs.len()), Some(1));
        assert!(
            plan.runtime
                .as_ref()
                .is_some_and(|rt| rt.active_job_id.is_none())
        );
        assert!(matches!(
            plan.runtime.as_ref().and_then(|rt| rt.followup.as_ref()),
            Some(crate::core::plan::PlanFollowup::Checkpoint { .. })
        ));
    }

    #[test]
    fn finish_active_plan_job_blocks_linked_step_on_failure() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::memory::storage::init_db(&conn).expect("init db");
        crate::memory::storage::save_plan_state(
            &conn,
            "plan-block-session",
            &PlanState {
                explanation: None,
                items: vec![PlanItem {
                    step: "Run migration".to_string(),
                    status: PlanStepStatus::InProgress,
                    job_id: None,
                }],
                runtime: None,
                updated_at: None,
            },
        )
        .expect("save plan");

        start_plan_job(
            &conn,
            "plan-block-session",
            "run_command",
            Some("cargo test".to_string()),
            PlanJobStatus::Running,
        )
        .expect("start job");
        finish_active_plan_job(&conn, "plan-block-session", PlanJobStatus::Failed)
            .expect("finish job");

        let plan = crate::memory::storage::load_plan_state(&conn, "plan-block-session")
            .expect("load plan")
            .expect("plan present");
        assert_eq!(plan.items[0].status, PlanStepStatus::Blocked);
        assert!(plan.items[0].job_id.is_none());
        assert!(matches!(
            plan.runtime.as_ref().and_then(|rt| rt.followup.as_ref()),
            Some(crate::core::plan::PlanFollowup::RetryOrReplan { retry_count: 1, .. })
        ));
    }

    #[test]
    fn finish_active_plan_job_persists_output_preview() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::memory::storage::init_db(&conn).expect("init db");
        crate::memory::storage::save_plan_state(
            &conn,
            "plan-output-session",
            &PlanState {
                explanation: None,
                items: vec![PlanItem {
                    step: "Run command".to_string(),
                    status: PlanStepStatus::InProgress,
                    job_id: None,
                }],
                runtime: None,
                updated_at: None,
            },
        )
        .expect("save plan");

        start_plan_job(
            &conn,
            "plan-output-session",
            "run_command",
            Some("cargo test".to_string()),
            PlanJobStatus::Running,
        )
        .expect("start job");
        finish_active_plan_job_with_output(
            &conn,
            "plan-output-session",
            PlanJobStatus::Succeeded,
            Some("ok\nall green".to_string()),
            false,
        )
        .expect("finish job");

        let plan = crate::memory::storage::load_plan_state(&conn, "plan-output-session")
            .expect("load plan")
            .expect("plan present");
        let runtime = plan.runtime.expect("runtime kept for job history");
        assert_eq!(runtime.jobs.len(), 1);
        assert_eq!(
            runtime.jobs[0].output_preview.as_deref(),
            Some("ok\nall green")
        );
        assert!(!runtime.jobs[0].has_error_output);
    }

    #[test]
    fn append_active_plan_job_output_updates_live_transcript_and_preview() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::memory::storage::init_db(&conn).expect("init db");
        crate::memory::storage::save_plan_state(
            &conn,
            "plan-live-output-session",
            &PlanState {
                explanation: None,
                items: vec![PlanItem {
                    step: "Run command".to_string(),
                    status: PlanStepStatus::InProgress,
                    job_id: None,
                }],
                runtime: None,
                updated_at: None,
            },
        )
        .expect("save plan");

        start_plan_job(
            &conn,
            "plan-live-output-session",
            "run_command",
            Some("cargo test".to_string()),
            PlanJobStatus::Running,
        )
        .expect("start job");
        append_active_plan_job_output(&conn, "plan-live-output-session", "line one\n", false)
            .expect("append stdout");
        append_active_plan_job_output(&conn, "plan-live-output-session", "line two\n", true)
            .expect("append stderr");

        let plan = crate::memory::storage::load_plan_state(&conn, "plan-live-output-session")
            .expect("load plan")
            .expect("plan present");
        let runtime = plan.runtime.expect("runtime kept for live output");
        let job = runtime.jobs.first().expect("job present");
        assert_eq!(job.output_transcript, "line one\nline two\n");
        assert_eq!(job.output_preview.as_deref(), Some("line one\nline two"));
        assert!(job.has_error_output);
    }

    #[test]
    fn set_plan_step_status_promotes_selected_step_to_in_progress() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::memory::storage::init_db(&conn).expect("init db");
        crate::memory::storage::save_plan_state(
            &conn,
            "plan-edit-session",
            &PlanState {
                explanation: None,
                items: vec![
                    PlanItem {
                        step: "First".to_string(),
                        status: PlanStepStatus::InProgress,
                        job_id: Some("job-1".to_string()),
                    },
                    PlanItem {
                        step: "Second".to_string(),
                        status: PlanStepStatus::Pending,
                        job_id: None,
                    },
                ],
                runtime: None,
                updated_at: None,
            },
        )
        .expect("save plan");

        set_plan_step_status(&conn, "plan-edit-session", 1, PlanStepStatus::InProgress)
            .expect("set status");

        let plan = crate::memory::storage::load_plan_state(&conn, "plan-edit-session")
            .expect("load plan")
            .expect("plan exists");
        assert_eq!(plan.items[0].status, PlanStepStatus::Pending);
        assert_eq!(plan.items[0].job_id, None);
        assert_eq!(plan.items[1].status, PlanStepStatus::InProgress);
    }

    #[test]
    fn clear_plan_state_removes_saved_plan() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::memory::storage::init_db(&conn).expect("init db");
        crate::memory::storage::save_plan_state(
            &conn,
            "plan-clear-session",
            &PlanState {
                explanation: None,
                items: vec![PlanItem {
                    step: "First".to_string(),
                    status: PlanStepStatus::Pending,
                    job_id: None,
                }],
                runtime: None,
                updated_at: None,
            },
        )
        .expect("save plan");

        clear_plan_state(&conn, "plan-clear-session").expect("clear plan");

        let plan = crate::memory::storage::load_plan_state(&conn, "plan-clear-session")
            .expect("load plan");
        assert!(plan.is_none());
    }

    #[test]
    fn clear_plan_followup_keeps_plan_items() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::memory::storage::init_db(&conn).expect("init db");
        crate::memory::storage::save_plan_state(
            &conn,
            "plan-followup-clear-session",
            &PlanState {
                explanation: None,
                items: vec![PlanItem {
                    step: "Inspect".to_string(),
                    status: PlanStepStatus::InProgress,
                    job_id: None,
                }],
                runtime: Some({
                    let mut runtime = crate::core::plan::PlanRuntime::default();
                    runtime.set_checkpoint_followup("Inspect", Some("Patch".to_string()));
                    runtime
                }),
                updated_at: None,
            },
        )
        .expect("save plan");

        clear_plan_followup(&conn, "plan-followup-clear-session").expect("clear followup");

        let plan = crate::memory::storage::load_plan_state(&conn, "plan-followup-clear-session")
            .expect("load plan")
            .expect("plan present");
        assert_eq!(plan.items.len(), 1);
        assert!(
            plan.runtime
                .as_ref()
                .is_none_or(|runtime| runtime.followup.is_none())
        );
    }

    #[test]
    fn replan_blocked_step_rewrites_plan_deterministically() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::memory::storage::init_db(&conn).expect("init db");
        crate::memory::storage::save_plan_state(
            &conn,
            "plan-replan-session",
            &PlanState {
                explanation: Some("Original plan".to_string()),
                items: vec![PlanItem {
                    step: "Patch failing handler".to_string(),
                    status: PlanStepStatus::Blocked,
                    job_id: None,
                }],
                runtime: Some({
                    let mut runtime = crate::core::plan::PlanRuntime::default();
                    runtime.set_retry_or_replan_followup(
                        "Patch failing handler",
                        Some("cargo test".to_string()),
                    );
                    runtime
                }),
                updated_at: None,
            },
        )
        .expect("save plan");

        replan_blocked_step(&conn, "plan-replan-session", 0).expect("replan step");

        let plan = crate::memory::storage::load_plan_state(&conn, "plan-replan-session")
            .expect("load plan")
            .expect("plan present");
        assert_eq!(plan.items.len(), 2);
        assert_eq!(plan.items[0].status, PlanStepStatus::InProgress);
        assert!(
            plan.items[0]
                .step
                .contains("Revise approach for blocked step: Patch failing handler")
        );
        assert_eq!(plan.items[1].status, PlanStepStatus::Pending);
        assert!(
            plan.items[1]
                .step
                .contains("Validate revised approach for: Patch failing handler")
        );
        assert!(
            plan.runtime
                .as_ref()
                .is_none_or(|runtime| runtime.followup.is_none())
        );
    }
}
