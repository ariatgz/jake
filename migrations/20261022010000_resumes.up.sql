-- One master resume (LaTeX source) per profile; tailored copies are built on demand.
CREATE TABLE resumes (
  user_id BIGINT PRIMARY KEY REFERENCES profiles(user_id) ON DELETE CASCADE,
  file_name TEXT NOT NULL,
  tex TEXT NOT NULL,
  max_pages SMALLINT NOT NULL DEFAULT 1,
  auto_tailor BOOLEAN NOT NULL DEFAULT TRUE, -- tailor automatically when Apply is clicked
  update_time TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  CONSTRAINT resume_max_pages_check CHECK (max_pages BETWEEN 1 AND 3)
);
