// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/ioctl.h>
#include <string.h>
#include <sys/mman.h>
#include <linux/fb.h>

#define GETVSCREENINFO 0x4600
#define PUTVSCREENINFO 0x4601
#define GETFSCREENINFO 0x4602

unsigned int colors[] = {
	// RGB value for colors (assuming 32-bit color depth)
	0xFF0000, // Red
	0x00FF00, // Green
	0x0000FF, // Blue
	0xFFFF00, // Yellow
	0xFF00FF, // Magenta
	0x00FFFF, // Cyan
	0xFFFFFF, // White
	0x000000 // Black
};
const unsigned int color_count = sizeof(colors) / sizeof(colors[0]);

int main()
{
	int fb_fd = open("/dev/fb0", O_RDWR);
	if (fb_fd < 0) {
		perror("Failed to open /dev/fb0");
		return 1;
	}

	struct fb_var_screeninfo var_info;
	if (ioctl(fb_fd, GETVSCREENINFO, &var_info) == 0) {
		printf("Framebuffer resolution: %ux%u, virtual: %ux%u, bpp: %u\n",
		       var_info.xres, var_info.yres, var_info.xres_virtual,
		       var_info.yres_virtual, var_info.bits_per_pixel);
	} else {
		perror("GETVSCREENINFO ioctl failed");
	}

	struct fb_fix_screeninfo fix_info;
	if (ioctl(fb_fd, GETFSCREENINFO, &fix_info) == 0) {
		printf("Framebuffer memory: start=0x%lx, length=%u bytes, line length=%u bytes\n",
		       fix_info.smem_start, fix_info.smem_len,
		       fix_info.line_length);
	} else {
		perror("GETFSCREENINFO ioctl failed");
	}

	for (unsigned int i = 0; i < color_count; i++) {
		void *fb_mem = mmap(NULL, fix_info.smem_len,
				    PROT_READ | PROT_WRITE, MAP_SHARED, fb_fd,
				    0);
		if (fb_mem == MAP_FAILED) {
			perror("Failed to mmap framebuffer memory");
		} else {
			printf("Framebuffer memory mapped at address %p\n",
			       fb_mem);

			//Fill the framebuffer with different colors
			if (var_info.bits_per_pixel == 32) {
				unsigned int *pixel = (unsigned int *)fb_mem;
				for (unsigned int y = 0; y < var_info.yres;
				     y++) {
					for (unsigned int x = 0;
					     x < var_info.xres; x++) {
						pixel[y * (fix_info.line_length /
							   4) +
						      x] = colors[i];
					}
				}
			} else {
				fprintf(stderr,
					"Unsupported bits per pixel: %u\n",
					var_info.bits_per_pixel);
			}
			sleep(3);
			//Unmap the framebuffer memory
			if (munmap(fb_mem, fix_info.smem_len) < 0) {
				perror("Failed to munmap framebuffer memory");
			}
		}
	}

	close(fb_fd);
	return 0;
}
