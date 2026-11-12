-- agentrix-records (W2): the courier a ready order was booked with, and its tracking number
-- (additive). v4, "Delivery is bought from another business": at ready the shop's agent books
-- the delivery at the courier's agent door; the courier owns the tracking, read live by this
-- number. Collection orders, and orders from before, keep NULL.
ALTER TABLE rec_orders ADD COLUMN courier TEXT;
ALTER TABLE rec_orders ADD COLUMN tracking TEXT;
