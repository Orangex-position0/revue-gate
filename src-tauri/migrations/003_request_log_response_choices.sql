-- Store model response choices in canonical OpenAI Chat form.
ALTER TABLE request_logs ADD COLUMN response_choices TEXT;
