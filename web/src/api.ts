export class ApiError extends Error {
  status: number;
  code: string;
  constructor(status: number, code: string, message: string) {
    super(message);
    this.status = status;
    this.code = code;
  }
}

let onUnauthorized = () => {};

export function setUnauthorized(handler: () => void) {
  onUnauthorized = handler;
}

export async function api<T>(path: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers);
  // 写操作 CSRF 第二道闸：同源 JS 标记，浏览器跨站表单带不上该头。
  headers.set("x-dlz-requested", "1");
  if (init.body && !(init.body instanceof FormData) && !headers.has("content-type")) {
    headers.set("content-type", "application/json");
  }
  const response = await fetch(path, { ...init, headers, credentials: "include" });
  const text = await response.text();
  const body = text ? JSON.parse(text) : null;
  if (!response.ok) {
    const error = new ApiError(response.status, body?.error ?? "http", body?.message ?? "请求失败");
    if (response.status === 401 && !path.startsWith("/api/session") && path !== "/api/register") {
      onUnauthorized();
    }
    throw error;
  }
  return body as T;
}

export async function apiBlob(path: string, init: RequestInit = {}): Promise<Blob> {
  const headers = new Headers(init.headers);
  headers.set("x-dlz-requested", "1");
  const response = await fetch(path, { ...init, headers, credentials: "include" });
  if (!response.ok) {
    const text = await response.text();
    let message = "请求失败";
    try {
      message = JSON.parse(text).message ?? message;
    } catch {
      /* 二进制失败时没有 JSON */
    }
    throw new ApiError(response.status, "http", message);
  }
  return response.blob();
}

// 登出：吊销当前会话并清 cookie；失败时调用方按 ApiError 处理。
export async function logout(): Promise<{ revoked: boolean }> {
  return api<{ revoked: boolean }>("/api/session/logout", { method: "POST" });
}

export type Kind = "菜" | "饭" | "汤";
export type Slot = "breakfast" | "lunch" | "dinner";

export interface Groups<T> {
  菜: T[];
  饭: T[];
  汤: T[];
}

export interface Me {
  id: string;
  username: string;
  display_name: string | null;
  email: string | null;
  email_verified: boolean;
  totp: boolean;
  roles: { role: string; canteen_ids: string[]; source_label: string | null }[];
  permissions: string[];
  email_enabled: boolean;
}

export function personName(
  me: { display_name?: string | null; username?: string | null } | null | undefined,
): string {
  const custom = me?.display_name?.trim();
  if (custom) return custom;
  return me?.username?.trim() ?? "";
}

export interface Canteen {
  id: string;
  name: string;
}

export interface Dish {
  id: string;
  name: string;
  kind: Kind;
  taste_tags: string[];
  image_url: string | null;
  enabled: boolean;
  canteen_id: string;
}

export interface Candidate {
  dish_id: string;
  name: string;
  kind: Kind;
  taste_tags: string[];
  image_url: string | null;
  selected: boolean;
  tone: string;
}

export const KINDS: Kind[] = ["菜", "饭", "汤"];
export const SLOTS: Slot[] = ["breakfast", "lunch", "dinner"];

export const SLOT_SHORT: Record<Slot, string> = {
  breakfast: "早",
  lunch: "中",
  dinner: "晚",
};

export const SLOT_NAME: Record<Slot, string> = {
  breakfast: "早餐",
  lunch: "午餐",
  dinner: "晚餐",
};

export function emptyGroups<T>(): Groups<T> {
  return { 菜: [], 饭: [], 汤: [] };
}

export function shanghaiToday(): string {
  return new Intl.DateTimeFormat("en-CA", {
    timeZone: "Asia/Shanghai",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  }).format(new Date());
}

export function addDays(iso: string, days: number): string {
  const [year, month, day] = iso.split("-").map(Number);
  const date = new Date(Date.UTC(year, month - 1, day));
  date.setUTCDate(date.getUTCDate() + days);
  return date.toISOString().slice(0, 10);
}

export function weekStart(iso: string): string {
  const [year, month, day] = iso.split("-").map(Number);
  const date = new Date(Date.UTC(year, month - 1, day));
  return addDays(iso, -((date.getUTCDay() + 6) % 7));
}

export function weekdayLabel(iso: string): string {
  const names = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];
  const [year, month, day] = iso.split("-").map(Number);
  const date = new Date(Date.UTC(year, month - 1, day));
  return names[(date.getUTCDay() + 6) % 7];
}

export function explain(error: unknown, fallback = "请求失败"): string {
  return error instanceof Error ? error.message : fallback;
}

export function toneColor(tone: string): string {
  switch (tone) {
    case "green":
      return "#2d8a45";
    case "lime":
      return "#7cb342";
    case "amber":
      return "#e6a23c";
    case "red":
      return "#e53935";
    default:
      return "#d1d5db";
  }
}
