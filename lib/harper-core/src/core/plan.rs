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

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepStatus {
    Pending,
    InProgress,
    Completed,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanItem {
    pub step: String,
    pub status: PlanStepStatus,
    #[serde(default)]
    pub job_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanJobStatus {
    WaitingApproval,
    Running,
    Blocked,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanLoopStage {
    Planning,
    Inspecting,
    Executing,
    Feedback,
    RetryPending,
    ReplanRequired,
    Responding,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanLoopOutcome {
    WaitingApproval,
    Succeeded,
    Failed,
    Checkpointed,
    RetryPending,
    ReplanRequired,
    Responded,
    BackendUnavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanJobRecord {
    pub job_id: String,
    pub tool: String,
    pub command: Option<String>,
    pub status: PlanJobStatus,
    #[serde(default)]
    pub output_transcript: String,
    #[serde(default)]
    pub output_preview: Option<String>,
    #[serde(default)]
    pub has_error_output: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlanFollowup {
    Checkpoint {
        step: String,
        next_step: Option<String>,
    },
    RetryOrReplan {
        step: String,
        command: Option<String>,
        retry_count: u32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthoringPhase {
    ContextBuilt,
    PlanCreated,
    FilesInspected,
    EditsApplied,
    Validated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AuthoringValidationStep {
    pub command: String,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AuthoringPlannedEdit {
    pub path: String,
    pub change: String,
    #[serde(default)]
    pub why: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct StructuredAuthoringPlan {
    #[serde(default)]
    pub primary_files: Vec<String>,
    #[serde(default)]
    pub supporting_files: Vec<String>,
    #[serde(default)]
    pub validation_files: Vec<String>,
    #[serde(default)]
    pub planned_edits: Vec<AuthoringPlannedEdit>,
    #[serde(default)]
    pub validation_plan: Vec<AuthoringValidationStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AuthoringRuntime {
    #[serde(default)]
    pub request: Option<String>,
    #[serde(default)]
    pub phase: Option<AuthoringPhase>,
    #[serde(default)]
    pub candidate_files: Vec<String>,
    #[serde(default)]
    pub inspected_files: Vec<String>,
    #[serde(default)]
    pub edit_scope: Vec<String>,
    #[serde(default)]
    pub structured_plan: Option<StructuredAuthoringPlan>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PlanRuntime {
    #[serde(default)]
    pub active_tool: Option<String>,
    #[serde(default)]
    pub active_command: Option<String>,
    #[serde(default)]
    pub active_status: Option<String>,
    #[serde(default)]
    pub active_job_id: Option<String>,
    #[serde(default)]
    pub jobs: Vec<PlanJobRecord>,
    #[serde(default)]
    pub followup: Option<PlanFollowup>,
    #[serde(default)]
    pub followup_history: Vec<PlanFollowup>,
    #[serde(default)]
    pub loop_stage: Option<PlanLoopStage>,
    #[serde(default)]
    pub last_outcome: Option<PlanLoopOutcome>,
    #[serde(default)]
    pub last_feedback: Option<String>,
    #[serde(default)]
    pub authoring: Option<AuthoringRuntime>,
}

impl PlanRuntime {
    pub fn has_active_state(&self) -> bool {
        self.active_tool.is_some() || self.active_command.is_some() || self.active_status.is_some()
    }

    pub fn is_empty(&self) -> bool {
        !self.has_active_state()
            && self.active_job_id.is_none()
            && self.jobs.is_empty()
            && self.followup.is_none()
            && self.followup_history.is_empty()
            && self.loop_stage.is_none()
            && self.last_outcome.is_none()
            && self.last_feedback.is_none()
            && self.authoring.is_none()
    }

    pub fn set_active_tool_state(
        &mut self,
        tool: impl Into<String>,
        command: Option<String>,
        status: impl Into<String>,
    ) {
        self.active_tool = Some(tool.into());
        self.active_command = command;
        self.active_status = Some(status.into());
        self.loop_stage = Some(PlanLoopStage::Executing);
    }

    pub fn set_loop_stage(&mut self, stage: PlanLoopStage, feedback: Option<String>) {
        self.loop_stage = Some(stage);
        if let Some(feedback) = feedback.filter(|value| !value.trim().is_empty()) {
            self.last_feedback = Some(feedback);
        }
    }

    pub fn record_outcome(&mut self, outcome: PlanLoopOutcome, feedback: Option<String>) {
        self.last_outcome = Some(outcome);
        if let Some(feedback) = feedback.filter(|value| !value.trim().is_empty()) {
            self.last_feedback = Some(feedback);
        }
    }

    pub fn start_job(
        &mut self,
        tool: impl Into<String>,
        command: Option<String>,
        status: PlanJobStatus,
    ) -> String {
        let tool = tool.into();
        let job_id = Uuid::new_v4().to_string();
        self.active_job_id = Some(job_id.clone());
        self.active_tool = Some(tool.clone());
        self.active_command = command.clone();
        self.active_status = Some(job_status_label(&status).to_string());
        self.loop_stage = Some(match status {
            PlanJobStatus::WaitingApproval => PlanLoopStage::RetryPending,
            PlanJobStatus::Running => PlanLoopStage::Executing,
            PlanJobStatus::Blocked | PlanJobStatus::Failed => PlanLoopStage::ReplanRequired,
            PlanJobStatus::Succeeded => PlanLoopStage::Feedback,
        });
        self.last_outcome = Some(match status {
            PlanJobStatus::WaitingApproval => PlanLoopOutcome::WaitingApproval,
            PlanJobStatus::Running => PlanLoopOutcome::Responded,
            PlanJobStatus::Blocked | PlanJobStatus::Failed => PlanLoopOutcome::Failed,
            PlanJobStatus::Succeeded => PlanLoopOutcome::Succeeded,
        });
        self.jobs.push(PlanJobRecord {
            job_id: job_id.clone(),
            tool,
            command,
            status,
            output_transcript: String::new(),
            output_preview: None,
            has_error_output: false,
        });
        self.archive_active_followup();
        job_id
    }

    pub fn update_active_job_status(&mut self, status: PlanJobStatus) {
        let Some(active_job_id) = self.active_job_id.as_deref() else {
            return;
        };
        if let Some(job) = self
            .jobs
            .iter_mut()
            .rev()
            .find(|job| job.job_id == active_job_id)
        {
            job.status = status.clone();
        }
        self.active_status = Some(job_status_label(&status).to_string());
        self.loop_stage = Some(match status {
            PlanJobStatus::WaitingApproval => PlanLoopStage::RetryPending,
            PlanJobStatus::Running => PlanLoopStage::Executing,
            PlanJobStatus::Blocked | PlanJobStatus::Failed => PlanLoopStage::ReplanRequired,
            PlanJobStatus::Succeeded => PlanLoopStage::Feedback,
        });
        self.last_outcome = Some(match status {
            PlanJobStatus::WaitingApproval => PlanLoopOutcome::WaitingApproval,
            PlanJobStatus::Running => PlanLoopOutcome::Responded,
            PlanJobStatus::Blocked | PlanJobStatus::Failed => PlanLoopOutcome::Failed,
            PlanJobStatus::Succeeded => PlanLoopOutcome::Succeeded,
        });
    }

    pub fn set_active_job_output(
        &mut self,
        output_preview: Option<String>,
        has_error_output: bool,
    ) {
        let Some(active_job_id) = self.active_job_id.as_deref() else {
            return;
        };
        if let Some(job) = self
            .jobs
            .iter_mut()
            .rev()
            .find(|job| job.job_id == active_job_id)
        {
            job.output_preview = output_preview;
            job.has_error_output = has_error_output;
        }
    }

    pub fn append_active_job_output(&mut self, chunk: &str, is_error: bool) {
        const MAX_JOB_TRANSCRIPT_CHARS: usize = 16 * 1024;

        let Some(active_job_id) = self.active_job_id.as_deref() else {
            return;
        };
        if let Some(job) = self
            .jobs
            .iter_mut()
            .rev()
            .find(|job| job.job_id == active_job_id)
        {
            job.output_transcript.push_str(chunk);
            if job.output_transcript.chars().count() > MAX_JOB_TRANSCRIPT_CHARS {
                let trimmed: String = job
                    .output_transcript
                    .chars()
                    .rev()
                    .take(MAX_JOB_TRANSCRIPT_CHARS)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                job.output_transcript = trimmed;
            }
            job.has_error_output |= is_error;
            job.output_preview = preview_text(&job.output_transcript, 512);
        }
    }

    pub fn finish_active_job(&mut self, status: PlanJobStatus) {
        self.update_active_job_status(status);
        self.loop_stage = Some(PlanLoopStage::Feedback);
        self.clear_active_state();
    }

    pub fn set_checkpoint_followup(&mut self, step: impl Into<String>, next_step: Option<String>) {
        let next_followup = PlanFollowup::Checkpoint {
            step: step.into(),
            next_step,
        };
        if self.followup.as_ref() != Some(&next_followup) {
            self.archive_active_followup();
        }
        self.followup = Some(next_followup);
        self.loop_stage = Some(PlanLoopStage::Feedback);
        self.last_outcome = Some(PlanLoopOutcome::Checkpointed);
        self.last_feedback = Some("checkpoint recorded".to_string());
    }

    pub fn set_retry_or_replan_followup(
        &mut self,
        step: impl Into<String>,
        command: Option<String>,
    ) {
        let step = step.into();
        let retry_count = match self.followup.as_ref() {
            Some(PlanFollowup::RetryOrReplan {
                step: existing_step,
                retry_count,
                ..
            }) if existing_step == &step => retry_count.saturating_add(1),
            _ => 1,
        };
        let next_followup = PlanFollowup::RetryOrReplan {
            step,
            command,
            retry_count,
        };
        let should_archive_existing = !matches!(
            self.followup.as_ref(),
            Some(PlanFollowup::RetryOrReplan { step: existing_step, .. })
                if existing_step == followup_step(&next_followup)
        );
        if should_archive_existing {
            self.archive_active_followup();
        }
        self.followup = Some(next_followup);
        if retry_count <= 1 {
            self.loop_stage = Some(PlanLoopStage::RetryPending);
            self.last_outcome = Some(PlanLoopOutcome::RetryPending);
            self.last_feedback = Some("retry suggested".to_string());
        } else {
            self.loop_stage = Some(PlanLoopStage::ReplanRequired);
            self.last_outcome = Some(PlanLoopOutcome::ReplanRequired);
            self.last_feedback = Some("replan required".to_string());
        }
    }

    pub fn clear_followup(&mut self) {
        self.archive_active_followup();
    }

    fn archive_active_followup(&mut self) {
        const MAX_FOLLOWUP_HISTORY: usize = 8;

        if let Some(current) = self.followup.take() {
            self.followup_history.push(current);
            if self.followup_history.len() > MAX_FOLLOWUP_HISTORY {
                let overflow = self.followup_history.len() - MAX_FOLLOWUP_HISTORY;
                self.followup_history.drain(0..overflow);
            }
        }
    }

    pub fn seed_authoring_context(
        &mut self,
        request: impl Into<String>,
        candidate_files: Vec<String>,
    ) {
        let mut candidate_files = candidate_files;
        candidate_files.sort();
        candidate_files.dedup();
        self.authoring = Some(AuthoringRuntime {
            request: Some(request.into()),
            phase: Some(AuthoringPhase::ContextBuilt),
            edit_scope: candidate_files.clone(),
            candidate_files,
            inspected_files: Vec::new(),
            structured_plan: None,
        });
    }

    pub fn mark_authoring_plan_created(&mut self) {
        let Some(authoring) = self.authoring.as_mut() else {
            return;
        };
        authoring.phase = Some(AuthoringPhase::PlanCreated);
    }

    pub fn set_authoring_structured_plan(&mut self, plan: StructuredAuthoringPlan) {
        if self.authoring.is_none() {
            let mut candidate_files = Vec::new();
            candidate_files.extend(plan.primary_files.iter().cloned());
            candidate_files.extend(plan.supporting_files.iter().cloned());
            candidate_files.extend(plan.validation_files.iter().cloned());
            candidate_files.extend(plan.planned_edits.iter().map(|edit| edit.path.clone()));
            candidate_files.sort();
            candidate_files.dedup();
            self.authoring = Some(AuthoringRuntime {
                request: None,
                phase: Some(AuthoringPhase::PlanCreated),
                candidate_files: candidate_files.clone(),
                inspected_files: Vec::new(),
                edit_scope: candidate_files,
                structured_plan: Some(plan),
            });
            return;
        }
        let Some(authoring) = self.authoring.as_mut() else {
            return;
        };
        let mut edit_scope = authoring.edit_scope.clone();
        edit_scope.extend(plan.primary_files.iter().cloned());
        edit_scope.extend(plan.supporting_files.iter().cloned());
        edit_scope.extend(plan.validation_files.iter().cloned());
        edit_scope.extend(plan.planned_edits.iter().map(|edit| edit.path.clone()));
        edit_scope.sort();
        edit_scope.dedup();
        authoring.edit_scope = edit_scope;
        authoring.structured_plan = Some(plan);
        authoring.phase = Some(AuthoringPhase::PlanCreated);
    }

    pub fn mark_authoring_inspection<I>(&mut self, paths: I)
    where
        I: IntoIterator<Item = String>,
    {
        let Some(authoring) = self.authoring.as_mut() else {
            return;
        };
        for path in paths {
            if !authoring
                .inspected_files
                .iter()
                .any(|existing| existing == &path)
            {
                authoring.inspected_files.push(path.clone());
            }
            if !authoring
                .edit_scope
                .iter()
                .any(|existing| existing == &path)
            {
                authoring.edit_scope.push(path);
            }
        }
        authoring.inspected_files.sort();
        authoring.inspected_files.dedup();
        authoring.edit_scope.sort();
        authoring.edit_scope.dedup();
        authoring.phase = Some(AuthoringPhase::FilesInspected);
    }

    pub fn mark_authoring_edit_applied<I>(&mut self, paths: I)
    where
        I: IntoIterator<Item = String>,
    {
        let Some(authoring) = self.authoring.as_mut() else {
            return;
        };
        for path in paths {
            if !authoring
                .edit_scope
                .iter()
                .any(|existing| existing == &path)
            {
                authoring.edit_scope.push(path);
            }
        }
        authoring.edit_scope.sort();
        authoring.edit_scope.dedup();
        authoring.phase = Some(AuthoringPhase::EditsApplied);
    }

    pub fn authoring_edit_scope(&self) -> Option<&[String]> {
        self.authoring
            .as_ref()
            .map(|authoring| authoring.edit_scope.as_slice())
    }

    pub fn authoring_inspected_files(&self) -> Option<&[String]> {
        self.authoring
            .as_ref()
            .map(|authoring| authoring.inspected_files.as_slice())
    }

    pub fn authoring_phase(&self) -> Option<&AuthoringPhase> {
        self.authoring
            .as_ref()
            .and_then(|authoring| authoring.phase.as_ref())
    }

    pub fn authoring_structured_plan(&self) -> Option<&StructuredAuthoringPlan> {
        self.authoring
            .as_ref()
            .and_then(|authoring| authoring.structured_plan.as_ref())
    }

    pub fn mark_authoring_validated(&mut self) {
        let Some(authoring) = self.authoring.as_mut() else {
            return;
        };
        authoring.phase = Some(AuthoringPhase::Validated);
    }

    pub fn clear_active_state(&mut self) {
        self.active_tool = None;
        self.active_command = None;
        self.active_status = None;
        self.active_job_id = None;
    }
}

fn job_status_label(status: &PlanJobStatus) -> &'static str {
    match status {
        PlanJobStatus::WaitingApproval => "waiting_approval",
        PlanJobStatus::Running => "running",
        PlanJobStatus::Blocked => "blocked",
        PlanJobStatus::Succeeded => "succeeded",
        PlanJobStatus::Failed => "failed",
    }
}

fn followup_step(followup: &PlanFollowup) -> &str {
    match followup {
        PlanFollowup::Checkpoint { step, .. } => step.as_str(),
        PlanFollowup::RetryOrReplan { step, .. } => step.as_str(),
    }
}

fn preview_text(text: &str, max_chars: usize) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    let preview: String = trimmed.chars().take(max_chars).collect();
    if trimmed.chars().count() > max_chars {
        Some(format!("{}…", preview))
    } else {
        Some(preview)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PlanState {
    pub explanation: Option<String>,
    pub items: Vec<PlanItem>,
    pub runtime: Option<PlanRuntime>,
    pub updated_at: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{
        AuthoringPhase, PlanFollowup, PlanJobStatus, PlanLoopOutcome, PlanLoopStage, PlanRuntime,
    };

    #[test]
    fn runtime_deserializes_legacy_shape_with_empty_jobs() {
        let runtime: PlanRuntime = serde_json::from_str(
            r#"{
                "active_tool":"run_command",
                "active_command":"echo hi",
                "active_status":"running"
            }"#,
        )
        .expect("legacy runtime json should deserialize");

        assert_eq!(runtime.active_tool.as_deref(), Some("run_command"));
        assert_eq!(runtime.active_command.as_deref(), Some("echo hi"));
        assert_eq!(runtime.active_status.as_deref(), Some("running"));
        assert!(runtime.active_job_id.is_none());
        assert!(runtime.jobs.is_empty());
    }

    #[test]
    fn runtime_tracks_job_lifecycle() {
        let mut runtime = PlanRuntime::default();
        let job_id = runtime.start_job(
            "run_command",
            Some("echo hi".to_string()),
            PlanJobStatus::Running,
        );

        assert_eq!(runtime.active_job_id.as_deref(), Some(job_id.as_str()));
        assert_eq!(runtime.jobs.len(), 1);
        assert_eq!(runtime.active_status.as_deref(), Some("running"));

        runtime.finish_active_job(PlanJobStatus::Succeeded);

        assert!(runtime.active_job_id.is_none());
        assert!(!runtime.has_active_state());
        assert_eq!(runtime.jobs[0].status, PlanJobStatus::Succeeded);
    }

    #[test]
    fn runtime_retry_followup_counts_repeated_failures() {
        let mut runtime = PlanRuntime::default();

        runtime.set_retry_or_replan_followup("Patch handler", Some("cargo test".to_string()));
        runtime.set_retry_or_replan_followup("Patch handler", Some("cargo test".to_string()));

        assert_eq!(
            runtime.followup,
            Some(PlanFollowup::RetryOrReplan {
                step: "Patch handler".to_string(),
                command: Some("cargo test".to_string()),
                retry_count: 2,
            })
        );
    }

    #[test]
    fn runtime_archives_followup_history_on_clear_and_replace() {
        let mut runtime = PlanRuntime::default();

        runtime.set_checkpoint_followup("Inspect output", Some("Patch handler".to_string()));
        runtime.clear_followup();
        runtime
            .set_retry_or_replan_followup("Retry failing command", Some("cargo test".to_string()));

        assert_eq!(runtime.followup_history.len(), 1);
        assert_eq!(
            runtime.followup_history[0],
            PlanFollowup::Checkpoint {
                step: "Inspect output".to_string(),
                next_step: Some("Patch handler".to_string())
            }
        );

        runtime.set_checkpoint_followup("Review failure", Some("Apply fix".to_string()));

        assert_eq!(runtime.followup_history.len(), 2);
        assert_eq!(
            runtime.followup_history[1],
            PlanFollowup::RetryOrReplan {
                step: "Retry failing command".to_string(),
                command: Some("cargo test".to_string()),
                retry_count: 1
            }
        );
    }

    #[test]
    fn runtime_tracks_loop_stage_and_outcome_for_retry_and_checkpoint() {
        let mut runtime = PlanRuntime::default();

        runtime.set_loop_stage(PlanLoopStage::Planning, Some("plan updated".to_string()));
        runtime.record_outcome(PlanLoopOutcome::Responded, Some("plan ready".to_string()));
        runtime
            .set_retry_or_replan_followup("Retry failing command", Some("cargo test".to_string()));
        assert_eq!(runtime.loop_stage, Some(PlanLoopStage::RetryPending));
        assert_eq!(runtime.last_outcome, Some(PlanLoopOutcome::RetryPending));

        runtime
            .set_retry_or_replan_followup("Retry failing command", Some("cargo test".to_string()));
        assert_eq!(runtime.loop_stage, Some(PlanLoopStage::ReplanRequired));
        assert_eq!(runtime.last_outcome, Some(PlanLoopOutcome::ReplanRequired));

        runtime.set_checkpoint_followup("Inspect output", Some("Patch handler".to_string()));
        assert_eq!(runtime.loop_stage, Some(PlanLoopStage::Feedback));
        assert_eq!(runtime.last_outcome, Some(PlanLoopOutcome::Checkpointed));
        assert_eq!(
            runtime.last_feedback.as_deref(),
            Some("checkpoint recorded")
        );
    }

    #[test]
    fn runtime_tracks_authoring_phase_and_scope() {
        let mut runtime = PlanRuntime::default();

        runtime.seed_authoring_context(
            "refactor planner flow",
            vec![
                "lib/harper-ui/src/interfaces/ui/widgets.rs".to_string(),
                "lib/harper-core/src/agent/chat.rs".to_string(),
            ],
        );
        assert_eq!(
            runtime.authoring_phase(),
            Some(&AuthoringPhase::ContextBuilt)
        );
        assert_eq!(
            runtime.authoring_edit_scope(),
            Some(
                &[
                    "lib/harper-core/src/agent/chat.rs".to_string(),
                    "lib/harper-ui/src/interfaces/ui/widgets.rs".to_string()
                ][..]
            )
        );

        runtime.mark_authoring_plan_created();
        assert_eq!(
            runtime.authoring_phase(),
            Some(&AuthoringPhase::PlanCreated)
        );

        runtime.mark_authoring_inspection(vec![
            "lib/harper-ui/src/interfaces/ui/widgets.rs".to_string()
        ]);
        assert_eq!(
            runtime.authoring_phase(),
            Some(&AuthoringPhase::FilesInspected)
        );
        assert_eq!(
            runtime.authoring_inspected_files(),
            Some(&["lib/harper-ui/src/interfaces/ui/widgets.rs".to_string()][..])
        );

        runtime.mark_authoring_edit_applied(vec!["lib/harper-core/src/tools/plan.rs".to_string()]);
        assert_eq!(
            runtime.authoring_phase(),
            Some(&AuthoringPhase::EditsApplied)
        );
        assert!(runtime
            .authoring_edit_scope()
            .expect("scope")
            .iter()
            .any(|path| path == "lib/harper-core/src/tools/plan.rs"));
    }
}
