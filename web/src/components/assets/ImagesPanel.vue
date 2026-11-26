<script setup lang="ts">
import { computed, onMounted, ref } from "vue";

import { deleteImage, fetchImages, imageRawUrl, uploadImages } from "../../api/assets";
import { ApiRequestError } from "../../api/http";
import type { ImageMeta, ImageScene } from "../../api/types";
import { useToast } from "../../composables/useToast";
import UiBadge from "../ui/UiBadge.vue";
import UiButton from "../ui/UiButton.vue";
import UiCard from "../ui/UiCard.vue";
import UiSelect from "../ui/UiSelect.vue";

const { push } = useToast();

const sceneOptions = [
  { value: "article_cover", label: "文章封面 (article_cover)" },
  { value: "avatar", label: "用户头像 (avatar)" },
  { value: "banner", label: "横幅海报 (banner)" },
] as const;

const filterOptions = [
  { value: "all", label: "全部场景" },
  ...sceneOptions,
] as const;

const selectedScene = ref<ImageScene>("article_cover");
const filterScene = ref("all");
const pendingFiles = ref<File[]>([]);
const fileInputRef = ref<HTMLInputElement | null>(null);

const loading = ref(false);
const uploading = ref(false);
const deleting = ref(false);
const images = ref<ImageMeta[]>([]);
const selectedIds = ref<string[]>([]);

const filteredImages = computed(() => {
  if (filterScene.value === "all") {
    return images.value;
  }
  return images.value.filter((img) => img.scene === filterScene.value);
});

const sceneCounts = computed(() => {
  let cover = 0;
  let avatar = 0;
  let banner = 0;
  for (const img of images.value) {
    if (img.scene === "article_cover") cover += 1;
    else if (img.scene === "avatar") avatar += 1;
    else if (img.scene === "banner") banner += 1;
  }
  return { cover, avatar, banner };
});

function parseScene(raw: string): ImageScene {
  if (raw === "avatar" || raw === "banner") {
    return raw;
  }
  return "article_cover";
}

async function loadImages(): Promise<void> {
  loading.value = true;
  try {
    const res = await fetchImages();
    images.value = res.items;
    selectedIds.value = selectedIds.value.filter((id) =>
      res.items.some((item) => item.id === id),
    );
  } catch (err) {
    push(err instanceof ApiRequestError ? err.message : "读取失败", "error");
  } finally {
    loading.value = false;
  }
}

function onFilesChanged(event: Event): void {
  const input = event.target;
  if (input instanceof HTMLInputElement && input.files) {
    pendingFiles.value = Array.from(input.files);
  }
}

function onDropFiles(event: DragEvent): void {
  const files = event.dataTransfer?.files;
  if (files && files.length > 0) {
    pendingFiles.value = Array.from(files);
  }
}

async function handleUpload(): Promise<void> {
  if (pendingFiles.value.length === 0) {
    return;
  }
  uploading.value = true;
  try {
    await uploadImages(selectedScene.value, pendingFiles.value);
    pendingFiles.value = [];
    if (fileInputRef.value) {
      fileInputRef.value.value = "";
    }
    push("导入成功", "success");
    await loadImages();
  } catch (err) {
    push(err instanceof ApiRequestError ? err.message : "导入失败", "error");
  } finally {
    uploading.value = false;
  }
}

async function handleDeleteOne(id: string): Promise<void> {
  deleting.value = true;
  try {
    await deleteImage(id);
    push("已删除", "info");
    await loadImages();
  } catch (err) {
    push(err instanceof ApiRequestError ? err.message : "删除失败", "error");
  } finally {
    deleting.value = false;
  }
}

async function handleBatchDelete(): Promise<void> {
  if (selectedIds.value.length === 0) {
    return;
  }
  deleting.value = true;
  try {
    for (const id of [...selectedIds.value]) {
      await deleteImage(id);
    }
    selectedIds.value = [];
    push("已删除", "info");
    await loadImages();
  } catch (err) {
    push(err instanceof ApiRequestError ? err.message : "删除失败", "error");
    await loadImages();
  } finally {
    deleting.value = false;
  }
}

function toggleSelect(id: string): void {
  if (selectedIds.value.includes(id)) {
    selectedIds.value = selectedIds.value.filter((item) => item !== id);
  } else {
    selectedIds.value = [...selectedIds.value, id];
  }
}

function formatBytes(size: number): string {
  if (size < 1024) {
    return `${size} B`;
  }
  if (size < 1024 * 1024) {
    return `${(size / 1024).toFixed(1)} KB`;
  }
  return `${(size / (1024 * 1024)).toFixed(2)} MB`;
}

onMounted(() => {
  void loadImages();
});
</script>

<template>
  <div class="space-y-4">
    <UiCard title="图片导入">
      <div
        class="grid gap-4 md:grid-cols-[220px_1fr_auto] md:items-end"
        @dragover.prevent
        @drop.prevent="onDropFiles"
      >
        <UiSelect
          :model-value="selectedScene"
          :options="sceneOptions"
          label="场景分类"
          test-id="image-scene-select"
          @update:model-value="(v) => (selectedScene = parseScene(v))"
        />

        <label class="block">
          <span class="mb-1.5 block text-xs font-medium text-slate-700">
            图片文件 (PNG / JPEG / GIF / WebP)
          </span>
          <input
            ref="fileInputRef"
            type="file"
            multiple
            accept="image/png,image/jpeg,image/gif,image/webp"
            data-testid="image-file-input"
            class="block w-full rounded-lg border border-dashed border-slate-300 bg-slate-50 px-3 py-1.5 text-xs text-slate-700 file:mr-3 file:rounded-md file:border-0 file:bg-slate-900 file:px-2.5 file:py-1 file:text-xs file:font-medium file:text-white hover:border-slate-400"
            @change="onFilesChanged"
          />
        </label>

        <div class="flex items-center gap-2">
          <span v-if="pendingFiles.length > 0" class="text-xs text-slate-500">
            {{ pendingFiles.length }} 张待传
          </span>
          <UiButton
            variant="primary"
            :disabled="pendingFiles.length === 0"
            :loading="uploading"
            test-id="image-upload-btn"
            @click="handleUpload"
          >
            导入
          </UiButton>
        </div>
      </div>
    </UiCard>

    <UiCard title="已导入素材">
      <template #actions>
        <div class="flex flex-wrap items-center gap-2">
          <UiBadge tone="neutral">封面 {{ sceneCounts.cover }}</UiBadge>
          <UiBadge tone="neutral">头像 {{ sceneCounts.avatar }}</UiBadge>
          <UiBadge tone="neutral">横幅 {{ sceneCounts.banner }}</UiBadge>
          <div class="w-44">
            <UiSelect
              v-model="filterScene"
              :options="filterOptions"
              test-id="image-filter-select"
            />
          </div>
          <UiButton
            v-if="selectedIds.length > 0"
            size="sm"
            variant="danger"
            :loading="deleting"
            test-id="image-batch-delete"
            @click="handleBatchDelete"
          >
            删除 ({{ selectedIds.length }})
          </UiButton>
          <UiButton size="sm" :loading="loading" @click="loadImages">
            刷新
          </UiButton>
        </div>
      </template>

      <div
        v-if="filteredImages.length === 0"
        class="py-10 text-center text-sm text-slate-500"
      >
        暂无图片素材
      </div>

      <div v-else class="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        <div
          v-for="img in filteredImages"
          :key="img.id"
          :data-testid="`image-item-${img.id}`"
          class="overflow-hidden rounded-lg border border-slate-200 bg-white"
        >
          <div class="relative aspect-video bg-slate-100">
            <img
              :src="imageRawUrl(img.id)"
              :alt="img.filename"
              loading="lazy"
              class="h-full w-full object-contain"
            />
            <input
              type="checkbox"
              :checked="selectedIds.includes(img.id)"
              class="absolute top-2 left-2 h-4 w-4 rounded border-slate-300"
              :aria-label="img.filename"
              @change="toggleSelect(img.id)"
            />
          </div>
          <div class="space-y-1.5 p-3 text-xs">
            <div class="flex items-center justify-between gap-2">
              <span class="truncate font-medium text-slate-900" :title="img.filename">
                {{ img.filename }}
              </span>
              <UiBadge tone="info">{{ img.scene }}</UiBadge>
            </div>
            <div class="flex items-center justify-between text-slate-500">
              <span>{{ img.width }}×{{ img.height }}</span>
              <span>{{ formatBytes(img.size) }}</span>
            </div>
            <div class="flex items-center justify-between gap-2 pt-1">
              <code class="truncate text-[11px] text-slate-500" :title="img.ahash">
                aHash {{ img.ahash }}
              </code>
              <UiButton
                size="sm"
                variant="danger"
                :disabled="deleting"
                :test-id="`image-delete-${img.id}`"
                @click="handleDeleteOne(img.id)"
              >
                删除
              </UiButton>
            </div>
          </div>
        </div>
      </div>
    </UiCard>
  </div>
</template>
