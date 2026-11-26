<script setup lang="ts">
import { SwitchRoot, SwitchThumb } from "reka-ui";

withDefaults(
  defineProps<{
    modelValue: boolean;
    disabled?: boolean;
    testId?: string;
    ariaLabel?: string;
  }>(),
  {
    disabled: false,
    testId: undefined,
    ariaLabel: undefined,
  },
);

const emit = defineEmits<{
  "update:modelValue": [value: boolean];
}>();

function onUpdate(next: boolean): void {
  emit("update:modelValue", next);
}
</script>

<template>
  <SwitchRoot
    :model-value="modelValue"
    :disabled="disabled"
    :data-testid="testId"
    :aria-label="ariaLabel"
    class="relative inline-flex h-6 w-11 shrink-0 cursor-pointer items-center rounded-full border border-transparent transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-slate-900 disabled:cursor-not-allowed disabled:opacity-45"
    :class="modelValue ? 'bg-emerald-600' : 'bg-slate-300'"
    @update:model-value="onUpdate"
  >
    <SwitchThumb
      class="pointer-events-none block h-5 w-5 rounded-full bg-white shadow-xs transition-transform"
      :class="modelValue ? 'translate-x-5' : 'translate-x-0.5'"
    />
  </SwitchRoot>
</template>
