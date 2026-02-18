-- Add thinking column to messages table for persisting reasoning/thinking blocks.
ALTER TABLE messages ADD COLUMN thinking TEXT;
