-include Makefile.options
lib=$(shell pwd)/ffmpeg-libs
ffmpeg_version=7.1.3
#####################################################################################
## print usage information
help:
	@echo 'Usage:'
	@cat ${MAKEFILE_LIST} | grep -e "^## " -A 1 | grep -v '\-\-' | sed 's/^##//' | cut -f1 -d":" | \
		awk '{info=$$0; getline; print "  " $$0 ": " info;}' | column -t -s ':' | sort 
.PHONY: help
###############################################################################
prepare: .done	
.PHONY: prepare
###############################################################################
FFmpeg:
	git clone --depth 1 --branch n$(ffmpeg_version) https://github.com/FFmpeg/FFmpeg.git
####################################################################################	
.done: FFmpeg
	mkdir -p $(lib)
	cd FFmpeg && \
	CC="zig cc" CXX="zig c++" AR="zig ar" ./configure \
	 --prefix=$(lib) \
	 --enable-gpl \
     --enable-version3 \
     --disable-shared \
     --enable-static \
     --disable-debug \
     --disable-doc \
     --enable-libmp3lame \
     --extra-cflags="-O3 -march=haswell" \
     --extra-ldflags="-static"
	cd FFmpeg && make -j12
	cd FFmpeg && make install	
	touch $@
####################################################################################
prepare/zig:
	curl -LO https://ziglang.org/download/0.15.2/zig-x86_64-linux-0.15.2.tar.xz \
		&& tar -xf zig-x86_64-linux-0.15.2.tar.xz \
		&& mv zig-x86_64-linux-0.15.2 /opt/zig \
		&& ln -s /opt/zig/zig /usr/local/bin/zig
####################################################################################
clean:
	rm -rf FFmpeg $(lib)
.PHONY: clean
####################################################################################

.EXPORT_ALL_VARIABLES:
