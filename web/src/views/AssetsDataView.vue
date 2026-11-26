<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";

import AppsPanel from "../components/assets/AppsPanel.vue";
import EnvelopesPanel from "../components/assets/EnvelopesPanel.vue";
import ImagesPanel from "../components/assets/ImagesPanel.vue";
import PostsPanel from "../components/assets/PostsPanel.vue";
import UiTabs from "../components/ui/UiTabs.vue";

const route = useRoute();
const router = useRouter();

const tabs = [
  { value: "images", label: "图片素材", testId: "tab-images" },
  { value: "posts", label: "帖子数据", testId: "tab-posts" },
  { value: "apps", label: "应用凭据", testId: "tab-apps" },
  { value: "envelopes", label: "信封密钥与画像", testId: "tab-envelopes" },
] as const;

function resolveTab(raw: unknown): string {
  if (
    raw === "images" ||
    raw === "posts" ||
    raw === "apps" ||
    raw === "envelopes"
  ) {
    return raw;
  }
  return "images";
}

const activeTab = ref(resolveTab(route.query.tab));

const initialEnvelopeFront = computed(() =>
  typeof route.query.front === "string" ? route.query.front : undefined,
);

watch(
  () => route.query.tab,
  (next) => {
    activeTab.value = resolveTab(next);
  },
);

function onTabChange(next: string): void {
  activeTab.value = next;
  void router.replace({ path: "/assets-data", query: { tab: next } });
}
</script>

<template>
  <div class="space-y-5">
    <UiTabs
      :model-value="activeTab"
      :items="tabs"
      @update:model-value="onTabChange"
    />

    <ImagesPanel v-if="activeTab === 'images'" />
    <PostsPanel v-else-if="activeTab === 'posts'" />
    <AppsPanel v-else-if="activeTab === 'apps'" />
    <EnvelopesPanel v-else :initial-front="initialEnvelopeFront" />
  </div>
</template>
