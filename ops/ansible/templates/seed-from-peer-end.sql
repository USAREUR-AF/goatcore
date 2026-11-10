DO $$
DECLARE saved record;
BEGIN
  FOR saved IN SELECT * FROM pg_temp.goat_seed_trigger_modes LOOP
    EXECUTE format('ALTER TABLE %s %s TRIGGER %I', saved.tgrelid::regclass,
      CASE saved.tgenabled
        WHEN 'A' THEN 'ENABLE ALWAYS'
        WHEN 'R' THEN 'ENABLE REPLICA'
        WHEN 'D' THEN 'DISABLE'
        WHEN 'O' THEN 'ENABLE'
      END, saved.tgname);
  END LOOP;
END $$;
COMMIT;
