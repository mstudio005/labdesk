-- SQLite schema for LabDesk. Money is integer paise; times are ISO-8601 UTC text.
-- Mirrors the data model of the web version, with the later ALTERs folded in.
CREATE TABLE IF NOT EXISTS users(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  username TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,
  password_hash TEXT NOT NULL,
  role TEXT NOT NULL CHECK(role IN ('admin','technician','pathologist','reception')),
  active INTEGER NOT NULL DEFAULT 1);
CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
-- Images the lab uploads (letterhead, signature). Kept inside the database so a backup contains everything.
CREATE TABLE IF NOT EXISTS assets(kind TEXT PRIMARY KEY, mime TEXT NOT NULL, data BLOB NOT NULL, updated_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS tests(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  code TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,
  department TEXT NOT NULL,
  sample TEXT NOT NULL,
  price_paise INTEGER NOT NULL CHECK(price_paise>=0),
  config TEXT NOT NULL,
  disabled INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS referrals(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL,
  phone TEXT NOT NULL DEFAULT '',
  email TEXT NOT NULL DEFAULT '',
  address TEXT NOT NULL DEFAULT '');
CREATE TABLE IF NOT EXISTS patients(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  patient_code TEXT NOT NULL UNIQUE,
  designation TEXT NOT NULL,
  first_name TEXT NOT NULL,
  last_name TEXT NOT NULL,
  age REAL NOT NULL,
  age_type TEXT NOT NULL CHECK(age_type IN ('Year','Month','Day')),
  gender TEXT NOT NULL,
  phone TEXT NOT NULL,
  email TEXT NOT NULL,
  address TEXT NOT NULL DEFAULT '');
CREATE TABLE IF NOT EXISTS orders(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  patient_id INTEGER NOT NULL REFERENCES patients(id),
  referral_id INTEGER REFERENCES referrals(id),
  referral_name TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'Pending' CHECK(status IN ('Pending','Partial','Completed','Delivered')),
  subtotal_paise INTEGER NOT NULL,
  discount_paise INTEGER NOT NULL,
  total_paise INTEGER NOT NULL,
  dispatch TEXT NOT NULL,
  version INTEGER NOT NULL DEFAULT 1,
  notes TEXT NOT NULL DEFAULT '',
  interpretation TEXT NOT NULL DEFAULT '',
  interpretation_position TEXT NOT NULL DEFAULT 'end',
  bill_note TEXT NOT NULL DEFAULT '',
  discount_reason TEXT NOT NULL DEFAULT '',
  approved_snapshot TEXT,
  approved_by INTEGER REFERENCES users(id),
  approved_at TEXT);
CREATE TABLE IF NOT EXISTS order_tests(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  order_id INTEGER NOT NULL REFERENCES orders(id),
  test_id INTEGER NOT NULL REFERENCES tests(id),
  snapshot TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'Pending',
  interpretation TEXT NOT NULL DEFAULT '',
  show_interpretation INTEGER NOT NULL DEFAULT 0,
  comment TEXT NOT NULL DEFAULT '',
  approved_snapshot TEXT,
  approved_at TEXT,
  approved_by_name TEXT,
  UNIQUE(order_id,test_id));
CREATE TABLE IF NOT EXISTS results(
  order_test_id INTEGER NOT NULL REFERENCES order_tests(id),
  parameter_id TEXT NOT NULL,
  value TEXT NOT NULL,
  flag TEXT NOT NULL DEFAULT '',
  range_text TEXT,
  comment TEXT NOT NULL DEFAULT '',
  manual INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY(order_test_id,parameter_id));
CREATE TABLE IF NOT EXISTS payments(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  order_id INTEGER NOT NULL REFERENCES orders(id),
  amount_paise INTEGER NOT NULL CHECK(amount_paise>0),
  method TEXT NOT NULL CHECK(method IN ('Cash','UPI','Card','Bank')),
  created_at TEXT NOT NULL,
  reference TEXT NOT NULL DEFAULT '');
CREATE TABLE IF NOT EXISTS audit(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  actor TEXT NOT NULL,
  action TEXT NOT NULL,
  entity TEXT NOT NULL,
  entity_id TEXT NOT NULL,
  details TEXT NOT NULL,
  created_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS report_versions(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  order_id INTEGER NOT NULL REFERENCES orders(id),
  version INTEGER NOT NULL,
  snapshot TEXT NOT NULL,
  reason TEXT NOT NULL,
  created_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS sequences(day TEXT PRIMARY KEY, value INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS idx_patients_name ON patients(first_name,last_name);
CREATE INDEX IF NOT EXISTS idx_orders_created ON orders(created_at);
CREATE INDEX IF NOT EXISTS idx_order_tests_order ON order_tests(order_id);
CREATE INDEX IF NOT EXISTS idx_payments_order ON payments(order_id);

-- The audit trail and delivered reports are append-only at the database level.
CREATE TRIGGER IF NOT EXISTS audit_no_update BEFORE UPDATE ON audit BEGIN SELECT RAISE(ABORT,'audit log is append-only'); END;
CREATE TRIGGER IF NOT EXISTS audit_no_delete BEFORE DELETE ON audit BEGIN SELECT RAISE(ABORT,'audit log is append-only'); END;
CREATE TRIGGER IF NOT EXISTS delivered_no_result_change BEFORE UPDATE ON results
  WHEN (SELECT o.status FROM order_tests t JOIN orders o ON o.id=t.order_id WHERE t.id=NEW.order_test_id)='Delivered'
  BEGIN SELECT RAISE(ABORT,'delivered reports are permanently locked'); END;
