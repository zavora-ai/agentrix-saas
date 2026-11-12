-- Delivery addresses and recipients a customer keeps on a site (01a A-20). The area must be
-- one a delivery zone covers; the zone is stored so checkout can price it.
CREATE TABLE rec_addresses (
    site TEXT NOT NULL,
    address_id TEXT NOT NULL,
    customer_id TEXT NOT NULL,
    recipient TEXT NOT NULL,
    phone TEXT,
    area TEXT NOT NULL,
    zone TEXT NOT NULL,
    details TEXT NOT NULL DEFAULT '',
    notes TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL,
    PRIMARY KEY (site, address_id)
);
CREATE INDEX rec_addresses_customer ON rec_addresses (site, customer_id, created_at);
