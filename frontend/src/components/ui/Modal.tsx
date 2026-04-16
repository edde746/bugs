import { Show, createEffect, onCleanup } from "solid-js";
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

// Stack of currently-open modals. Only the topmost entry handles Escape,
// so nested/stacked modals close one at a time instead of all responding
// to a single keypress. A single module-level keydown listener forwards
// Escape to the top of the stack; modals don't add per-instance listeners.
type CloseHandler = () => void;
const modalStack: CloseHandler[] = [];

if (typeof document !== "undefined") {
  document.addEventListener("keydown", (e: KeyboardEvent) => {
    if (e.key !== "Escape") return;
    const top = modalStack[modalStack.length - 1];
    if (top) top();
  });
}

export default function Modal(props: ModalProps) {
  let handler: CloseHandler | null = null;

  // Register/unregister with the stack whenever `open` flips. Using a
  // reactive effect here (instead of onMount) means a single <Modal>
  // component that toggles open state multiple times still behaves
  // correctly — each close removes the handler, each open re-pushes it.
  createEffect(() => {
    if (props.open) {
      handler = () => props.onClose();
      modalStack.push(handler);
      onCleanup(() => {
        if (handler) {
          const idx = modalStack.lastIndexOf(handler);
          if (idx >= 0) modalStack.splice(idx, 1);
          handler = null;
        }
      });
    }
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
