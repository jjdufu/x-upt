<script setup lang="ts">
import { TabsList, TabsRoot, TabsTrigger } from "reka-ui";

export interface TabItem {
  value: string;
  label: string;
  testId?: string;
  count?: number;
}

defineProps<{
  modelValue: string;
  items: ReadonlyArray<TabItem>;
}>();

const emit = defineEmits<{
  "update:modelValue": [value: string];
}>();

function onUpdate(next: string | number): void {
  emit("update:modelValue", String(next));
}
</script>

<template>
  <TabsRoot :model-value="modelValue" @update:model-value="onUpdate">
    <TabsList
      class="inline-flex flex-wrap items-center gap-1 rounded-lg border border-slate-200 bg-slate-100 p-1"
    >
      <TabsTrigger
        v-for="item in items"
        :key="item.value"
        :value="item.value"
        :data-testid="item.testId"
        class="inline-flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-medium transition-colors data-[state=active]:bg-white data-[state=active]:text-slate-900 data-[state=active]:shadow-2xs data-[state=inactive]:text-slate-600 hover:text-slate-900"
      >
        <span>{{ item.label }}</span>
        <span
          v-if="item.count !== undefined"
          class="rounded-full bg-slate-200/80 px-1.5 py-0.5 text-[11px] leading-none text-slate-700"
        >
          {{ item.count }}
        </span>
      </TabsTrigger>
    </TabsList>
    <slot />
  </TabsRoot>
</template>
