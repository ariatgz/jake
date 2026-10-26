-- Telegram chat that also receives this profile's job alerts; NULL = Discord only.
ALTER TABLE profiles ADD COLUMN telegram_chat_id BIGINT NULL;
