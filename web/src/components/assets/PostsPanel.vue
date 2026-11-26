<script setup lang="ts">
import { onMounted, reactive, ref } from "vue";

import {
  createPost,
  deletePost,
  fetchPosts,
  importPosts,
  updatePost,
} from "../../api/assets";
import { ApiRequestError, isNumber, isRecord, isString } from "../../api/http";
import type { PostInput, PostRecord } from "../../api/types";
import { useToast } from "../../composables/useToast";
import UiBadge from "../ui/UiBadge.vue";
import UiButton from "../ui/UiButton.vue";
import UiCard from "../ui/UiCard.vue";
import UiDialog from "../ui/UiDialog.vue";
import UiInput from "../ui/UiInput.vue";

const { push } = useToast();

const loading = ref(false);
const keyword = ref("");
const page = ref(1);
const pageSize = ref(50);
const total = ref(0);
const items = ref<PostRecord[]>([]);

const editOpen = ref(false);
const saving = ref(false);
const editingId = ref<number | null>(null);
const postForm = reactive({
  id: "",
  userId: 1,
  title: "",
  body: "",
});

const importOpen = ref(false);
const importing = ref(false);
const importJsonText = ref("");

function isPostInputItem(value: unknown): value is PostInput {
  if (!isRecord(value)) {
    return false;
  }
  if (value.id !== undefined && !isNumber(value.id)) {
    return false;
  }
  return isNumber(value.userId) && isString(value.title) && isString(value.body);
}

async function loadPosts(): Promise<void> {
  loading.value = true;
  try {
    const res = await fetchPosts(page.value, pageSize.value, keyword.value.trim());
    items.value = res.items;
    total.value = res.total;
  } catch (err) {
    push(err instanceof ApiRequestError ? err.message : "读取失败", "error");
  } finally {
    loading.value = false;
  }
}

function openCreate(): void {
  editingId.value = null;
  postForm.id = "";
  postForm.userId = 1;
  postForm.title = "";
  postForm.body = "";
  editOpen.value = true;
}

function openEdit(post: PostRecord): void {
  editingId.value = post.id;
  postForm.id = String(post.id);
  postForm.userId = post.userId;
  postForm.title = post.title;
  postForm.body = post.body;
  editOpen.value = true;
}

async function handleSavePost(): Promise<void> {
  saving.value = true;
  try {
    const payload: PostInput = {
      userId: Number(postForm.userId),
      title: postForm.title.trim(),
      body: postForm.body.trim(),
    };
    if (editingId.value === null) {
      if (postForm.id.trim().length > 0) {
        payload.id = Number(postForm.id);
      }
      await createPost(payload);
    } else {
      await updatePost(editingId.value, payload);
    }
    editOpen.value = false;
    push("已保存", "success");
    await loadPosts();
  } catch (err) {
    push(err instanceof ApiRequestError ? err.message : "保存失败", "error");
  } finally {
    saving.value = false;
  }
}

async function handleDeletePost(id: number): Promise<void> {
  try {
    await deletePost(id);
    push("已删除", "info");
    await loadPosts();
  } catch (err) {
    push(err instanceof ApiRequestError ? err.message : "删除失败", "error");
  }
}

async function onJsonFileSelected(event: Event): Promise<void> {
  const input = event.target;
  if (input instanceof HTMLInputElement && input.files && input.files[0]) {
    importJsonText.value = await input.files[0].text();
  }
}

async function handleBatchImport(): Promise<void> {
  importing.value = true;
  try {
    const parsed: unknown = JSON.parse(importJsonText.value);
    const arr = Array.isArray(parsed)
      ? parsed
      : isRecord(parsed) && Array.isArray(parsed.items)
        ? parsed.items
        : null;
    if (!arr || !arr.every(isPostInputItem)) {
      push("JSON 格式不匹配", "error");
      return;
    }
    await importPosts(arr);
    importOpen.value = false;
    importJsonText.value = "";
    push("导入成功", "success");
    await loadPosts();
  } catch (err) {
    push(err instanceof ApiRequestError ? err.message : "导入失败", "error");
  } finally {
    importing.value = false;
  }
}

function handleSearch(): void {
  page.value = 1;
  void loadPosts();
}

onMounted(() => {
  void loadPosts();
});
</script>

<template>
  <div class="space-y-4">
    <UiCard title="帖子数据">
      <template #actions>
        <div class="flex flex-wrap items-center gap-2">
          <UiBadge tone="neutral">总数 {{ total }}</UiBadge>
          <div class="w-48">
            <UiInput
              v-model="keyword"
              type="search"
              placeholder="gRPC"
              test-id="post-keyword-input"
              @keyup.enter="handleSearch"
            />
          </div>
          <UiButton size="sm" test-id="post-refresh-btn" :loading="loading" @click="handleSearch">
            刷新
          </UiButton>
          <UiButton size="sm" test-id="post-import-open" @click="importOpen = true">
            批量导入
          </UiButton>
          <UiButton size="sm" variant="primary" test-id="post-create-open" @click="openCreate">
            新建
          </UiButton>
        </div>
      </template>

      <div v-if="items.length === 0" class="py-10 text-center text-sm text-slate-500">
        暂无帖子记录
      </div>

      <div v-else class="overflow-x-auto">
        <table class="min-w-full divide-y divide-slate-200 text-left text-xs">
          <thead class="bg-slate-50 text-slate-600">
            <tr>
              <th class="px-3 py-2 font-medium">ID</th>
              <th class="px-3 py-2 font-medium">用户 ID</th>
              <th class="px-3 py-2 font-medium">标题</th>
              <th class="px-3 py-2 font-medium">正文摘要</th>
              <th class="px-3 py-2 text-right font-medium">操作</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-slate-100">
            <tr v-for="p in items" :key="p.id" :data-testid="`post-row-${p.id}`">
              <td class="px-3 py-2 font-mono text-slate-700">{{ p.id }}</td>
              <td class="px-3 py-2 font-mono text-slate-700">{{ p.userId }}</td>
              <td class="max-w-xs truncate px-3 py-2 font-medium text-slate-900">
                {{ p.title }}
              </td>
              <td class="max-w-md truncate px-3 py-2 text-slate-500">
                {{ p.body }}
              </td>
              <td class="px-3 py-2 text-right">
                <div class="inline-flex items-center gap-1.5">
                  <UiButton
                    size="sm"
                    variant="ghost"
                    :test-id="`post-edit-${p.id}`"
                    @click="openEdit(p)"
                  >
                    编辑
                  </UiButton>
                  <UiButton
                    size="sm"
                    variant="danger"
                    :test-id="`post-delete-${p.id}`"
                    @click="handleDeletePost(p.id)"
                  >
                    删除
                  </UiButton>
                </div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <div
        v-if="total > pageSize"
        class="mt-4 flex items-center justify-between border-t border-slate-100 pt-3 text-xs text-slate-600"
      >
        <span>第 {{ page }} 页</span>
        <div class="flex gap-2">
          <UiButton
            size="sm"
            :disabled="page <= 1"
            @click="
              page -= 1;
              loadPosts();
            "
          >
            上一页
          </UiButton>
          <UiButton
            size="sm"
            :disabled="page * pageSize >= total"
            @click="
              page += 1;
              loadPosts();
            "
          >
            下一页
          </UiButton>
        </div>
      </div>
    </UiCard>

    <UiDialog
      v-model:open="editOpen"
      :title="editingId === null ? '新建帖子' : `编辑帖子 #${editingId}`"
    >
      <form class="space-y-3" @submit.prevent="handleSavePost">
        <div class="grid gap-3 sm:grid-cols-2">
          <UiInput
            v-if="editingId === null"
            v-model="postForm.id"
            type="number"
            :min="1"
            label="帖子 ID (可选)"
            placeholder="1"
          />
          <UiInput
            :model-value="postForm.userId"
            type="number"
            :min="1"
            label="用户 ID"
            placeholder="1"
            test-id="post-userid-input"
            @update:model-value="(v) => (postForm.userId = Number(v))"
          />
        </div>
        <UiInput
          v-model="postForm.title"
          label="标题"
          placeholder="Sample Post Title"
          test-id="post-title-input"
        />
        <label class="block">
          <span class="mb-1.5 block text-xs font-medium text-slate-700">正文</span>
          <textarea
            v-model="postForm.body"
            rows="4"
            placeholder="Deterministic post content"
            data-testid="post-body-input"
            class="block w-full rounded-lg border border-slate-200 p-2.5 text-sm focus:border-slate-900 focus:outline-none"
          />
        </label>
        <div class="flex justify-end gap-2 pt-2">
          <UiButton @click="editOpen = false">取消</UiButton>
          <UiButton
            type="submit"
            variant="primary"
            :loading="saving"
            test-id="post-save-btn"
          >
            保存
          </UiButton>
        </div>
      </form>
    </UiDialog>

    <UiDialog v-model:open="importOpen" title="批量导入帖子" max-width="xl">
      <form class="space-y-3" @submit.prevent="handleBatchImport">
        <label class="block">
          <span class="mb-1.5 block text-xs font-medium text-slate-700">JSON 文件</span>
          <input
            type="file"
            accept=".json,application/json"
            class="block w-full text-xs text-slate-600 file:mr-3 file:rounded-md file:border-0 file:bg-slate-900 file:px-2.5 file:py-1 file:text-xs file:text-white"
            @change="onJsonFileSelected"
          />
        </label>
        <label class="block">
          <span class="mb-1.5 block text-xs font-medium text-slate-700">JSON 数组</span>
          <textarea
            v-model="importJsonText"
            rows="8"
            :placeholder="'[{&quot;id&quot;:1,&quot;userId&quot;:1,&quot;title&quot;:&quot;Sample&quot;,&quot;body&quot;:&quot;Content&quot;}]'"
            data-testid="post-import-textarea"
            class="block w-full rounded-lg border border-slate-200 p-2.5 font-mono text-xs focus:border-slate-900 focus:outline-none"
          />
        </label>
        <div class="flex justify-end gap-2 pt-2">
          <UiButton @click="importOpen = false">取消</UiButton>
          <UiButton
            type="submit"
            variant="primary"
            :loading="importing"
            test-id="post-import-submit"
          >
            导入
          </UiButton>
        </div>
      </form>
    </UiDialog>
  </div>
</template>
