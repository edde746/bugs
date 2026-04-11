import { Show, onCleanup, onMount } from "solid-js";
import type { JSX } from "solid-js";
import { Portal } from "solid-js/web";
import IconX from "~icons/lucide/x";

interface ModalProps {
  open: boolean;
  onClose: () => void;
  title: string;
  description?: string;
  children: JSX.Element;
  footer?: JSX.Element;
}

export default function Modal(props: ModalProps) {
  const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape") props.onClose();
  };

  onMount(() => {
    document.addEventListener("keydown", handleKeyDown);
  });

  onCleanup(() => {
    document.removeEventListener("keydown", handleKeyDown);
  });

  return (
    <Show when={props.open}>
      <Portal>
        <div class="modal-overlay" onClick={(e) => {
          if (e.target === e.currentTarget) props.onClose();
        }}>
          <div class="modal" role="dialog" aria-modal="true">
            <div class="modal__header">
              <div>
                <h2 class="modal__title">{props.title}</h2>
                <Show when={props.description}>
                  <p class="modal__description">{props.description}</p>
                </Show>
              </div>
              <button class="modal__close" onClick={props.onClose} aria-label="Close">
                <IconX />
              </button>
            </div>
            <div class="modal__body">
              {props.children}
            </div>
            <Show when={props.footer}>
              <div class="modal__footer">
                {props.footer}
              </div>
            </Show>
          </div>
        </div>
      </Portal>
    </Show>
  );
}
