use axum::{
    extract::{Query, State},
    middleware::from_fn_with_state,
    response::Json as ResponseJson,
    routing::{get, post},
    Extension, Json, Router,
};
use db::models::{
    project::Project,
    task::{CreateTask, Task, TaskWithAttemptStatus, UpdateTask},
    task_attempt::{CreateTaskAttempt, TaskAttempt, TaskAttemptError},
};
use deployment::Deployment;
use serde::Deserialize;
use services::services::{container::ContainerService, git::GitService};
use sqlx::Error as SqlxError;
use utils::response::ApiResponse;
use uuid::Uuid;

use crate::{error::ApiError, middleware::load_task_middleware, DeploymentImpl};

#[derive(Debug, Deserialize)]
pub struct TaskQuery {
    pub project_id: Uuid,
}

pub async fn get_tasks(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<TaskQuery>,
) -> Result<ResponseJson<ApiResponse<Vec<TaskWithAttemptStatus>>>, ApiError> {
    let tasks =
        Task::find_by_project_id_with_attempt_status(&deployment.db().pool, query.project_id)
            .await?;

    Ok(ResponseJson(ApiResponse::success(tasks)))
}

pub async fn get_task(
    Extension(task): Extension<Task>,
    State(_deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<Task>>, ApiError> {
    Ok(ResponseJson(ApiResponse::success(task)))
}

pub async fn create_task(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<CreateTask>,
) -> Result<ResponseJson<ApiResponse<Task>>, ApiError> {
    let id = Uuid::new_v4();

    tracing::debug!(
        "Creating task '{}' in project {}",
        payload.title,
        payload.project_id
    );

    let task = Task::create(&deployment.db().pool, &payload, id).await?;

    // Track task creation event
    deployment
        .track_if_analytics_allowed(
            "task_created",
            serde_json::json!({
            "task_id": task.id.to_string(),
            "project_id": payload.project_id,
            "has_description": task.description.is_some(),
            }),
        )
        .await;

    Ok(ResponseJson(ApiResponse::success(task)))
}

pub async fn create_task_and_start(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<CreateTask>,
) -> Result<ResponseJson<ApiResponse<TaskWithAttemptStatus>>, ApiError> {
    // create the task first
    let task_id = Uuid::new_v4();
    let task = Task::create(&deployment.db().pool, &payload, task_id).await?;
    deployment
        .track_if_analytics_allowed(
            "task_created",
            serde_json::json!({
                "task_id": task.id.to_string(),
                "project_id": task.project_id,
                "has_description": task.description.is_some(),
            }),
        )
        .await;

    // use the default executor profile and the current branch for the task attempt
    let default_profile_variant = deployment.config().read().await.profile.clone();
    let project = Project::find_by_id(&deployment.db().pool, payload.project_id)
        .await?
        .ok_or(ApiError::Database(SqlxError::RowNotFound))?;
    let branch = GitService::new().get_current_branch(&project.git_repo_path)?;
    let base_coding_agent = executors::command::AgentProfiles::get_cached()
        .get_profile(&default_profile_variant.profile)
        .map(|profile| profile.agent.to_string())
        .ok_or_else(|| {
            ApiError::TaskAttempt(TaskAttemptError::ValidationError(format!(
                "Profile not found: {:?}",
                default_profile_variant
            )))
        })?;

    let task_attempt = TaskAttempt::create(
        &deployment.db().pool,
        &CreateTaskAttempt {
            base_coding_agent: base_coding_agent.clone(),
            base_branch: branch,
        },
        task.id,
    )
    .await?;
    let execution_process = deployment
        .container()
        .start_attempt(&task_attempt, default_profile_variant.clone())
        .await?;
    deployment
        .track_if_analytics_allowed(
            "task_attempt_started",
            serde_json::json!({
                "task_id": task.id.to_string(),
                "base_coding_agent": &base_coding_agent,
                "profile": &default_profile_variant,
                "attempt_id": task_attempt.id.to_string(),
            }),
        )
        .await;

    let task = Task::find_by_id(&deployment.db().pool, task.id)
        .await?
        .ok_or(ApiError::Database(SqlxError::RowNotFound))?;

    tracing::info!("Started execution process {}", execution_process.id);
    Ok(ResponseJson(ApiResponse::success(TaskWithAttemptStatus {
        id: task.id,
        title: task.title,
        description: task.description,
        project_id: task.project_id,
        status: task.status,
        parent_task_attempt: task.parent_task_attempt,
        created_at: task.created_at,
        updated_at: task.updated_at,
        has_in_progress_attempt: true,
        has_merged_attempt: false,
        last_attempt_failed: false,
        base_coding_agent: task_attempt.base_coding_agent,
    })))
}

pub async fn update_task(
    Extension(existing_task): Extension<Task>,
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<UpdateTask>,
) -> Result<ResponseJson<ApiResponse<Task>>, ApiError> {
    // Use existing values if not provided in update
    let title = payload.title.unwrap_or(existing_task.title);
    let description = payload.description.or(existing_task.description);
    let status = payload.status.unwrap_or(existing_task.status);
    let parent_task_attempt = payload
        .parent_task_attempt
        .or(existing_task.parent_task_attempt);

    let task = Task::update(
        &deployment.db().pool,
        existing_task.id,
        existing_task.project_id,
        title,
        description,
        status,
        parent_task_attempt,
    )
    .await?;

    Ok(ResponseJson(ApiResponse::success(task)))
}

pub async fn delete_task(
    Extension(task): Extension<Task>,
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<()>>, ApiError> {
    let attempts = TaskAttempt::fetch_all(&deployment.db().pool, Some(task.id))
        .await
        .unwrap_or_default();
    // Delete all attempts including their containers
    for attempt in attempts {
        deployment
            .container()
            .delete(&attempt)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(
                    "Failed to delete task attempt {} for task {}: {}",
                    attempt.id,
                    task.id,
                    e
                );
            });
    }
    let rows_affected = Task::delete(&deployment.db().pool, task.id).await?;

    if rows_affected == 0 {
        Err(ApiError::Database(SqlxError::RowNotFound))
    } else {
        Ok(ResponseJson(ApiResponse::success(())))
    }
}

pub fn router(deployment: &DeploymentImpl) -> Router<DeploymentImpl> {
    let task_id_router = Router::new()
        .route("/", get(get_task).put(update_task).delete(delete_task))
        .layer(from_fn_with_state(deployment.clone(), load_task_middleware));

    let inner = Router::new()
        .route("/", get(get_tasks).post(create_task))
        .route("/create-and-start", post(create_task_and_start))
        .nest("/{task_id}", task_id_router);

    // mount under /projects/:project_id/tasks
    Router::new().nest("/tasks", inner)
}
