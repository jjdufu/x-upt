<script setup lang="ts">
withDefaults(
  defineProps<{
    tone?: "success" | "neutral" | "warning" | "danger" | "info";
    testId?: string;
  }>(),
  {
    tone: "neutral",
    testId: undefined,
  },
);
</script>

<template>
  <span
    :data-testid="testId"
    class="inline-flex items-center gap-1 rounded-full px-2.5 py-0.5 text-xs font-medium ring-1 ring-inset"
    :class="[
      tone === 'success'
        ? 'bg-emerald-50 text-emerald-700 ring-emerald-600/20'
        : tone === 'warning'
          ? 'bg-amber-50 text-amber-800 ring-amber-600/25'
          : tone === 'danger'
            ? 'bg-rose-50 text-rose-700 ring-rose-600/20'
            : tone === 'info'
              ? 'bg-sky-50 text-sky-700 ring-sky-600/20'
              : 'bg-slate-100 text-slate-600 ring-slate-500/15',
    ]"
  >
    <slot />
  </span>
</template>
