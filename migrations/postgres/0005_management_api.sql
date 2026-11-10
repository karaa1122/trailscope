-- Prices and invoices leave the service (decision 072): selling Trailscope
-- Cloud is the operator's business, kept in their own admin tools, which
-- manage organizations and plans through the management API. A plan here is
-- only a tier of limits.
DROP TABLE invoices;
ALTER TABLE plans DROP COLUMN price_iqd;
