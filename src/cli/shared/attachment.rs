use std::io::Write as _;
use std::path::{Path, PathBuf};

use miette::IntoDiagnostic as _;
use serde_json::json;
use yt::YtError;
use yt::api::Parent;
use yt::api::attachments::Attachment;
use yt::output::{Format, Table};

use crate::cli::Ctx;

pub(crate) fn attachment_json(attachment: &Attachment) -> serde_json::Value {
    json!({
        "id": attachment.id,
        "name": attachment.name,
        "size": attachment.size,
        "mime_type": attachment.mime_type,
        "author": attachment.author.as_ref().map(|author| author.login.as_str()),
        "created": attachment.created.map(yt::time::utc),
    })
}

pub(crate) async fn list(ctx: &Ctx, parent: Parent) -> yt::Result<()> {
    let client = ctx.client().await?;
    let attachments = yt::api::attachments::list(client, &parent).await?;
    match ctx.output.format() {
        Format::Table => {
            let mut table = Table::new(["NAME", "SIZE", "AUTHOR", "CREATED"]);
            for attachment in &attachments {
                table.row([
                    attachment.name.clone(),
                    attachment
                        .size
                        .map(|size| size.to_string())
                        .unwrap_or_default(),
                    attachment
                        .author
                        .as_ref()
                        .map(|author| author.login.clone())
                        .unwrap_or_default(),
                    attachment.created.map(yt::time::local).unwrap_or_default(),
                ]);
            }
            ctx.output.table(&table)
        }
        Format::Json => ctx
            .output
            .json(&attachments.iter().map(attachment_json).collect::<Vec<_>>()),
        Format::Ndjson => attachments
            .iter()
            .try_for_each(|attachment| ctx.output.ndjson(&attachment_json(attachment))),
    }
}

pub(crate) async fn upload(ctx: &Ctx, parent: Parent, files: Vec<PathBuf>) -> yt::Result<()> {
    let client = ctx.client().await?;
    let id = parent.id().to_string();
    let uploaded = yt::api::attachments::upload(client, &parent, &files).await?;
    match ctx.output.format() {
        Format::Table => {
            let names: Vec<&str> = uploaded
                .iter()
                .map(|attachment| attachment.name.as_str())
                .collect();
            ctx.note(&format!("Uploaded {} to {id}", names.join(", ")));
            Ok(())
        }
        Format::Json => ctx
            .output
            .json(&uploaded.iter().map(attachment_json).collect::<Vec<_>>()),
        Format::Ndjson => uploaded
            .iter()
            .try_for_each(|attachment| ctx.output.ndjson(&attachment_json(attachment))),
    }
}

pub(crate) async fn download(
    ctx: &Ctx,
    parent: Parent,
    name: String,
    output: Option<String>,
) -> yt::Result<()> {
    let client = ctx.client().await?;
    let attachments = yt::api::attachments::list(client, &parent).await?;
    let attachment = yt::api::attachments::find(&attachments, &name, &parent)?;
    if output.as_deref() == Some("-") {
        let data = yt::api::attachments::download(client, attachment).await?;
        return ctx.output.bytes(&data);
    }
    let path = match &output {
        Some(path) if std::path::Path::new(path).is_dir() => {
            std::path::Path::new(path).join(file_name(&attachment.name)?)
        }
        Some(path) => PathBuf::from(path),
        None => PathBuf::from(file_name(&attachment.name)?),
    };
    let existing = std::fs::symlink_metadata(&path).ok();
    if existing
        .as_ref()
        .is_some_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(YtError::Usage {
            message: format!(
                "{} is a symbolic link; pass -o with the path it points to",
                path.display()
            ),
        }
        .into());
    }
    if existing.is_some() {
        ctx.ui
            .confirm_destructive(&format!("overwrite {}", path.display()))
            .await?;
    }
    let data = yt::api::attachments::download(client, attachment).await?;
    let target = path.clone();
    let overwrite = existing.is_some();
    tokio::task::spawn_blocking(move || save(&target, &data, overwrite))
        .await
        .into_diagnostic()?
        .map_err(|error| miette::miette!("cannot save {}: {error}", path.display()))?;
    match ctx.output.format() {
        Format::Table => {
            ctx.note(&format!("Saved {}", path.display()));
            Ok(())
        }
        _ => ctx.output.json(&json!({
            "id": attachment.id,
            "name": attachment.name,
            "path": path.display().to_string(),
        })),
    }
}

pub(crate) async fn delete(ctx: &Ctx, parent: Parent, name: String) -> yt::Result<()> {
    let client = ctx.client().await?;
    let id = parent.id().to_string();
    let attachments = yt::api::attachments::list(client, &parent).await?;
    let attachment = yt::api::attachments::find(&attachments, &name, &parent)?;
    ctx.ui
        .confirm_destructive(&format!("delete {} from {id}", attachment.name))
        .await?;
    yt::api::attachments::delete(client, &parent, &attachment.id).await?;
    match ctx.output.format() {
        Format::Table => {
            ctx.note(&format!("Deleted {} from {id}", attachment.name));
            Ok(())
        }
        Format::Json => ctx.output.json(&[&attachment.id]),
        Format::Ndjson => ctx.output.ndjson(&attachment.id),
    }
}

/// The attachment's own name as a file in the current directory; never a path elsewhere.
fn file_name(name: &str) -> yt::Result<String> {
    let base = std::path::Path::new(name)
        .file_name()
        .map(|base| base.to_string_lossy().into_owned())
        .unwrap_or_default();
    if base.is_empty() || base == "." || base == ".." {
        return Err(YtError::Usage {
            message: format!("'{name}' cannot be used as a file name; pass -o PATH"),
        }
        .into());
    }
    if base.starts_with('.') {
        return Err(YtError::Usage {
            message: format!("'{name}' would be saved as a hidden file; pass -o PATH"),
        }
        .into());
    }
    Ok(base)
}

/// Creates the file anew, so a symbolic link that appears at `path` is never followed.
fn save(path: &Path, data: &[u8], overwrite: bool) -> std::io::Result<()> {
    if overwrite {
        std::fs::remove_file(path)?;
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(data)
}
