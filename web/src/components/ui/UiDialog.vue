<script setup lang="ts">
import {
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogOverlay,
  DialogPortal,
  DialogRoot,
  DialogTitle,
} from "reka-ui";

withDefaults(
  defineProps<{
    open: boolean;
    title: string;
    description?: string;
    maxWidth?: "md" | "lg" | "xl";
    testId?: string;
  }>(),
  {
    description: undefined,
    maxWidth: "lg",
    testId: undefined,
  },
);

const emit = defineEmits<{
  "update:open": [value: boolean];
}>();

function onOpenChange(next: boolean): void {
  emit("update:open", next);
}
</script>

<template>
  <DialogRoot :open="open" @update:open="onOpenChange">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-40 bg-slate-900/40 backdrop-blur-[1px]" />
      <DialogContent
        :data-testid="testId"
        class="fixed top-1/2 left-1/2 z-50 max-h-[88vh] w-[calc(100vw-2rem)] -translate-x-1/2 -translate-y-1/2 overflow-y-auto rounded-xl border border-slate-200 bg-white p-6 shadow-xl focus:outline-none"
        :class="
          maxWidth === 'md'
            ? 'max-w-md'
            : maxWidth === 'xl'
              ? 'max-w-3xl'
              : 'max-w-xl'
        "
      >
        <div class="mb-4 flex items-start justify-between gap-4">
          <div>
            <DialogTitle class="text-base font-semibold text-slate-900">
              {{ title }}
            </DialogTitle>
            <DialogDescription
              v-if="description"
              class="mt-1 text-xs text-slate-500"
            >
              {{ description }}
            </DialogDescription>
          </div>
          <DialogClose
            class="rounded-md p-1 text-slate-400 hover:bg-slate-100 hover:text-slate-700"
            aria-label="关闭"
          >
            <svg class="h-4 w-4" viewBox="0 0 20 20" fill="currentColor" aria-hidden="true">
              <path
                d="M6.28 5.22a.75.75 0 00-1.06 1.06L8.94 10l-3.72 3.72a.75.75 0 101.06 1.06L10 11.06l3.72 3.72a.75.75 0 101.06-1.06L11.06 10l3.72-3.72a.75.75 0 00-1.06-1.06L10 8.94 6.28 5.22z"
              />
            </svg>
          </DialogClose>
        </div>
        <slot />
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
