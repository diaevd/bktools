default:
        @just -l

publish *FLAGS:
	cargo ws publish --no-individual-tags {{FLAGS}} patch

