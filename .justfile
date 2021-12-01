default:
        @just -l

publish *FLAGS:
	cargo ws publish --no-individual-tags {{FLAGS}} patch
bkhdd:
	./target/debug/bkhdd ~/backup/bk/MKT-BK0011m-HDD.hdi

