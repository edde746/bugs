import { createQuery } from "@tanstack/solid-query";
import { createEffect, createSignal, For, Show } from "solid-js";
import { api } from "~/api/client";
import { queryKeys } from "~/queries/keys";
import type { EventAttachment } from "~/lib/sentry-types";
import Button from "~/components/ui/Button";
import IconDownload from "~icons/lucide/download";
import IconEye from "~icons/lucide/eye";
import IconEyeOff from "~icons/lucide/eye-off";
import IconPaperclip from "~icons/lucide/paperclip";

interface AttachmentsPanelProps {
  eventId: number;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value >= 10 ? 0 : 1)} ${units[unit]}`;
}

function downloadFilename(name: string): string {
  const sanitized = name.replace(/[\\/\0-\x1f\x7f]+/g, "_").trim();
  return sanitized || "attachment";
}

export default function AttachmentsPanel(props: AttachmentsPanelProps) {
  const [previewId, setPreviewId] = createSignal<number | null>(null);
  const [downloadingId, setDownloadingId] = createSignal<number | null>(null);

  createEffect(() => {
    props.eventId;
    setPreviewId(null);
  });

  const attachmentsQuery = createQuery(() => ({
    queryKey: queryKeys.events.attachments(props.eventId),
    queryFn: ({ signal }) =>
      api.get<EventAttachment[]>(
        `/internal/events/${props.eventId}/attachments`,
        signal,
      ),
  }));

  const previewQuery = createQuery(() => ({
    queryKey: queryKeys.events.attachmentText(
      props.eventId,
      previewId() ?? "none",
    ),
    queryFn: ({ signal }) =>
      api.text(
        `/internal/events/${props.eventId}/attachments/${previewId()}/text`,
        signal,
      ),
    enabled: previewId() !== null,
  }));

  const attachments = () => attachmentsQuery.data ?? [];
  const selectedAttachment = () =>
    attachments().find((attachment) => attachment.id === previewId()) ?? null;

  const downloadAttachment = async (attachment: EventAttachment) => {
    setDownloadingId(attachment.id);
    try {
      const blob = await api.blob(
        `/internal/events/${props.eventId}/attachments/${attachment.id}/download`,
      );
      const url = URL.createObjectURL(blob);
      const link = document.createElement("a");
      link.href = url;
      link.download = downloadFilename(attachment.name);
      document.body.appendChild(link);
      link.click();
      link.remove();
      URL.revokeObjectURL(url);
    } finally {
      setDownloadingId(null);
    }
  };

  return (
    <Show when={attachments().length > 0}>
      <div class="card attachments">
        <div class="card__header">
          <h3 class="inline-gap">
            <IconPaperclip /> Attachments
          </h3>
          <span class="text-xs text-secondary">
            {attachments().length} file{attachments().length === 1 ? "" : "s"}
          </span>
        </div>
        <table class="data-table data-table--compact attachments__table">
          <thead>
            <tr>
              <th>Name</th>
              <th>Type</th>
              <th>Size</th>
              <th data-align="right">Actions</th>
            </tr>
          </thead>
          <tbody>
            <For each={attachments()}>
              {(attachment) => (
                <tr>
                  <td>
                    <div class="attachments__name">{attachment.name}</div>
                    <Show when={attachment.attachment_type}>
                      <div class="text-secondary">
                        {attachment.attachment_type}
                      </div>
                    </Show>
                  </td>
                  <td>{attachment.content_type ?? "application/octet-stream"}</td>
                  <td>{formatBytes(attachment.size)}</td>
                  <td data-align="right">
                    <div class="attachments__actions">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() =>
                          setPreviewId((current) =>
                            current === attachment.id ? null : attachment.id,
                          )
                        }
                      >
                        {previewId() === attachment.id ? <IconEyeOff /> : <IconEye />} Plaintext
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        disabled={downloadingId() === attachment.id}
                        onClick={() => void downloadAttachment(attachment)}
                      >
                        <IconDownload /> Download
                      </Button>
                    </div>
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
        <Show when={previewId() !== null}>
          <div class="attachment-preview">
            <div class="attachment-preview__header">
              <span>{selectedAttachment()?.name ?? "Attachment"}</span>
              <span class="text-secondary">Plaintext preview</span>
            </div>
            <pre class="attachment-preview__content">
              <Show
                when={!previewQuery.isError}
                fallback="Unable to load attachment preview."
              >
                <Show when={!previewQuery.isPending} fallback="Loading attachment...">
                  {previewQuery.data ?? ""}
                </Show>
              </Show>
            </pre>
          </div>
        </Show>
      </div>
    </Show>
  );
}
