default:
        @just -l

publish *FLAGS:
	cargo ws publish --no-individual-tags {{FLAGS}} patch

bkhdd:
	./target/debug/bkhdd ./data/MKT-BK0011m-HDD.hdi

mkdos:
	./target/debug/fuse-mkdosfs ./data/MKDOS317.IMG ./data/mnt --auto-unmount

