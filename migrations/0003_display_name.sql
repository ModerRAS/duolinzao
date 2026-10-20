-- 登录账号 username 保持不变。显示名可空，界面优先展示非空显示名。
ALTER TABLE account ADD COLUMN display_name TEXT;

ALTER TABLE account
    ADD CONSTRAINT account_display_name_len
    CHECK (display_name IS NULL OR char_length(display_name) BETWEEN 1 AND 64);
