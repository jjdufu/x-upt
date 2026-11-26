<script setup lang="ts">
withDefaults(
  defineProps<{
    variant?: "primary" | "secondary" | "danger" | "ghost";
    size?: "sm" | "md";
    type?: "button" | "submit";
    disabled?: boolean;
    loading?: boolean;
    testId?: string;
  }>(),
  {
    variant: "secondary",
    size: "md",
    type: "button",
    disabled: false,
    loading: false,
    testId: undefined,
  },
);

const emit = defineEmits<{
  click: [event: MouseEvent];
}>();

function handleClick(event: MouseEvent): void {
  emit("click", event);
}
</script>

<template>
  <button
    :type="type"
    :disabled="disabled || loading"
    :data-testid="testId"
    class="inline-flex items-center justify-center gap-1.5 rounded-lg font-medium transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-slate-900 disabled:cursor-not-allowed disabled:opacity-50"
    :class="[
      size === 'sm' ? 'px-2.5 py-1.5 text-xs' : 'px-3.5 py-2 text-sm',
      variant === 'primary'
        ? 'bg-slate-900 text-white hover:bg-slate-800'
        : variant === 'danger'
          ? 'border border-rose-200 bg-rose-50 text-rose-700 hover:bg-rose-100'
          : variant === 'ghost'
            ? 'text-slate-600 hover:bg-slate-100 hover:text-slate-900'
            : 'border border-slate-200 bg-white text-slate-700 hover:bg-slate-50',
    ]"
    @click="handleClick"
  >
    <svg
      v-if="loading"
      class="h-3.5 w-3.5 animate-spin"
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
    >
      <circle
        class="opacity-25"
        cx="12"
        cy="12"
        r="10"
        stroke="currentColor"
        stroke-width="3"
      />
      <path
        class="opacity-75"
        fill="currentColor"
        d="M4 12a8 8 0 018-8v3a5 5 0 00-5 5H4z"
      />
    </svg>
    <slot />
  </button>
</template>
