<script setup lang="ts">
withDefaults(
  defineProps<{
    title?: string;
    testId?: string;
    padding?: "sm" | "md";
  }>(),
  {
    title: undefined,
    testId: undefined,
    padding: "md",
  },
);
</script>

<template>
  <section
    :data-testid="testId"
    class="rounded-xl border border-slate-200 bg-white shadow-2xs"
    :class="padding === 'sm' ? 'p-4' : 'p-5'"
  >
    <header
      v-if="title || $slots.actions"
      class="mb-4 flex flex-wrap items-center justify-between gap-3"
    >
      <h2 v-if="title" class="text-sm font-semibold text-slate-900">
        {{ title }}
      </h2>
      <div v-if="$slots.actions" class="flex items-center gap-2">
        <slot name="actions" />
      </div>
    </header>
    <slot />
  </section>
</template>
