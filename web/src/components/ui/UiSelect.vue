<script setup lang="ts">
export interface SelectOption {
  value: string;
  label: string;
}

withDefaults(
  defineProps<{
    modelValue: string;
    options: ReadonlyArray<SelectOption>;
    label?: string;
    disabled?: boolean;
    testId?: string;
  }>(),
  {
    label: undefined,
    disabled: false,
    testId: undefined,
  },
);

const emit = defineEmits<{
  "update:modelValue": [value: string];
}>();

function onChange(event: Event): void {
  const target = event.target;
  if (target instanceof HTMLSelectElement) {
    emit("update:modelValue", target.value);
  }
}
</script>

<template>
  <label class="block">
    <span v-if="label" class="mb-1.5 block text-xs font-medium text-slate-700">
      {{ label }}
    </span>
    <div class="relative">
      <select
        :value="modelValue"
        :disabled="disabled"
        :data-testid="testId"
        class="block w-full appearance-none rounded-lg border border-slate-200 bg-white py-1.5 pr-8 pl-3 text-sm text-slate-900 focus:border-slate-900 focus:outline-none disabled:bg-slate-50 disabled:text-slate-400"
        @change="onChange"
      >
        <option
          v-for="opt in options"
          :key="opt.value"
          :value="opt.value"
        >
          {{ opt.label }}
        </option>
      </select>
      <svg
        class="pointer-events-none absolute top-1/2 right-2.5 h-4 w-4 -translate-y-1/2 text-slate-400"
        viewBox="0 0 20 20"
        fill="currentColor"
        aria-hidden="true"
      >
        <path
          fill-rule="evenodd"
          d="M5.23 7.21a.75.75 0 011.06.02L10 11.168l3.71-3.938a.75.75 0 111.08 1.04l-4.25 4.5a.75.75 0 01-1.08 0l-4.25-4.5a.75.75 0 01.02-1.06z"
          clip-rule="evenodd"
        />
      </svg>
    </div>
  </label>
</template>
