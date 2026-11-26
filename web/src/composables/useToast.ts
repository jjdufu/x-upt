import { readonly, ref } from "vue";

export interface ToastItem {
  id: number;
  tone: "success" | "error" | "info";
  text: string;
}

const items = ref<ToastItem[]>([]);
let nextId = 1;

export function useToast() {
  function push(text: string, tone: ToastItem["tone"] = "info"): void {
    const id = nextId;
    nextId += 1;
    items.value = [...items.value, { id, tone, text }];
    window.setTimeout(() => {
      dismiss(id);
    }, 3600);
  }

  function dismiss(id: number): void {
    items.value = items.value.filter((item) => item.id !== id);
  }

  return {
    toasts: readonly(items),
    push,
    dismiss,
  };
}
