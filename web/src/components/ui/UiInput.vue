<script setup lang="ts">
withDefaults(
  defineProps<{
    modelValue: string | number;
    type?: "text" | "number" | "password" | "search";
    label?: string;
    placeholder?: string;
    disabled?: boolean;
    min?: number;
    max?: number;
    testId?: string;
  }>(),
  {
    type: "text",
    label: undefined,
    placeholder: undefined,
    disabled: false,
    min: undefined,
    max: undefined,
    testId: undefined,
  },
);

const emit = defineEmits<{
  "update:modelValue": [value: string];
}>();

function onInput(event: Event): void {
  const target = event.target;
  if (target instanceof HTMLInputElement) {
    emit("update:modelValue", target.value);
  }
}
</script>

<template>
  <label class="block">
    <span v-if="label" class="mb-1.5 block text-xs font-medium text-slate-700">
      {{ label }}
    </span>
    <input
      :type="type"
      :value="modelValue"
      :placeholder="placeholder"
      :disabled="disabled"
      :min="min"
      :max="max"
      :data-testid="testId"
      class="block w-full rounded-lg border border-slate-200 bg-white px-3 py-1.5 text-sm text-slate-900 placeholder:text-slate-400 focus:border-slate-900 focus:outline-none disabled:bg-slate-50 disabled:text-slate-400"
      @input="onInput"
    />
  </label>
</template>
