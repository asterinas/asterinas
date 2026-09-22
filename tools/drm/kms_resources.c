// SPDX-License-Identifier: MPL-2.0

#include <ctype.h>
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <xf86drm.h>
#include <xf86drmMode.h>

#define DEFAULT_DRM_DEVICE "/dev/dri/card0"

static int print_device_version(int fd) {
  drmVersionPtr version = drmGetVersion(fd);

  if (!version) {
    fprintf(stderr, "cannot get DRM device version: %s\n", strerror(errno));
    return -1;
  }

  printf("DRM device: %.*s\n", version->name_len,
         version->name ? version->name : "");
  printf("Device version: %d.%d.%d\n", version->version_major,
         version->version_minor, version->version_patchlevel);
  if (version->desc && version->desc_len > 0) {
    printf("Description: %.*s\n\n", version->desc_len, version->desc);
  }

  drmFreeVersion(version);
  return 0;
}

static const char *connection_name(drmModeConnection connection) {
  switch (connection) {
  case DRM_MODE_CONNECTED:
    return "connected";
  case DRM_MODE_DISCONNECTED:
    return "disconnected";
  case DRM_MODE_UNKNOWNCONNECTION:
    return "unknown";
  default:
    return "invalid";
  }
}

static const char *encoder_type_name(uint32_t type) {
  switch (type) {
  case DRM_MODE_ENCODER_NONE:
    return "none";
  case DRM_MODE_ENCODER_DAC:
    return "DAC";
  case DRM_MODE_ENCODER_TMDS:
    return "TMDS";
  case DRM_MODE_ENCODER_LVDS:
    return "LVDS";
  case DRM_MODE_ENCODER_TVDAC:
    return "TVDAC";
  case DRM_MODE_ENCODER_VIRTUAL:
    return "virtual";
  case DRM_MODE_ENCODER_DSI:
    return "DSI";
  case DRM_MODE_ENCODER_DPMST:
    return "DPMST";
  case DRM_MODE_ENCODER_DPI:
    return "DPI";
  default:
    return "unknown";
  }
}

static void print_mode(const drmModeModeInfo *mode, const char *indent) {
  printf("%s\"%s\" %ux%u@%u clock=%u kHz flags=%#x type=%#x\n", indent,
         mode->name, mode->hdisplay, mode->vdisplay, mode->vrefresh,
         mode->clock, mode->flags, mode->type);
  printf("%s  h: %u %u %u %u, v: %u %u %u %u, vscan=%u\n", indent,
         mode->hdisplay, mode->hsync_start, mode->hsync_end, mode->htotal,
         mode->vdisplay, mode->vsync_start, mode->vsync_end, mode->vtotal,
         mode->vscan);
}

static void print_id_list(const char *name, const uint32_t *ids, int count) {
  printf("  %s (%d):", name, count);
  for (int i = 0; i < count; i++) {
    printf(" %u", ids[i]);
  }
  putchar('\n');
}

static int print_properties(int fd, const uint32_t *property_ids,
                            const uint64_t *property_values,
                            uint32_t property_count, const char *indent) {
  int result = 0;

  printf("%sproperties (%u):\n", indent, property_count);
  for (uint32_t i = 0; i < property_count; i++) {
    drmModePropertyPtr property = drmModeGetProperty(fd, property_ids[i]);

    if (!property) {
      fprintf(stderr, "%s  property %u: %s\n", indent, property_ids[i],
              strerror(errno));
      result = -1;
      continue;
    }

    printf("%s  %u %s = %" PRIu64 " flags=%#x\n", indent, property->prop_id,
           property->name, property_values[i], property->flags);
    drmModeFreeProperty(property);
  }

  return result;
}

static int print_object_properties(int fd, uint32_t object_id,
                                   uint32_t object_type, const char *indent) {
  drmModeObjectPropertiesPtr properties =
      drmModeObjectGetProperties(fd, object_id, object_type);
  int result;

  if (!properties) {
    fprintf(stderr, "%sproperties unavailable: %s\n", indent, strerror(errno));
    return -1;
  }

  result = print_properties(fd, properties->props, properties->prop_values,
                            properties->count_props, indent);
  drmModeFreeObjectProperties(properties);
  return result;
}

static int print_crtcs(int fd, const drmModeRes *resources) {
  int result = 0;

  puts("\nCRTCs:");
  for (int i = 0; i < resources->count_crtcs; i++) {
    drmModeCrtcPtr crtc = drmModeGetCrtc(fd, resources->crtcs[i]);

    if (!crtc) {
      fprintf(stderr, "  CRTC %u: %s\n", resources->crtcs[i], strerror(errno));
      result = -1;
      continue;
    }

    printf("  CRTC %u: fb=%u position=%u,%u gamma=%u mode=%s\n", crtc->crtc_id,
           crtc->buffer_id, crtc->x, crtc->y, crtc->gamma_size,
           crtc->mode_valid ? "valid" : "none");
    if (crtc->mode_valid) {
      print_mode(&crtc->mode, "    ");
    }
    if (print_object_properties(fd, crtc->crtc_id, DRM_MODE_OBJECT_CRTC,
                                "    ") < 0) {
      result = -1;
    }
    drmModeFreeCrtc(crtc);
  }

  return result;
}

static int print_encoders(int fd, const drmModeRes *resources) {
  int result = 0;

  puts("\nEncoders:");
  for (int i = 0; i < resources->count_encoders; i++) {
    drmModeEncoderPtr encoder = drmModeGetEncoder(fd, resources->encoders[i]);

    if (!encoder) {
      fprintf(stderr, "  encoder %u: %s\n", resources->encoders[i],
              strerror(errno));
      result = -1;
      continue;
    }

    printf("  encoder %u: type=%s(%u) crtc=%u "
           "possible_crtcs=%#x possible_clones=%#x\n",
           encoder->encoder_id, encoder_type_name(encoder->encoder_type),
           encoder->encoder_type, encoder->crtc_id, encoder->possible_crtcs,
           encoder->possible_clones);
    drmModeFreeEncoder(encoder);
  }

  return result;
}

static int print_connectors(int fd, const drmModeRes *resources) {
  int result = 0;

  puts("\nConnectors:");
  for (int i = 0; i < resources->count_connectors; i++) {
    drmModeConnectorPtr connector =
        drmModeGetConnector(fd, resources->connectors[i]);
    const char *type_name;

    if (!connector) {
      fprintf(stderr, "  connector %u: %s\n", resources->connectors[i],
              strerror(errno));
      result = -1;
      continue;
    }

    type_name = drmModeGetConnectorTypeName(connector->connector_type);
    printf("  connector %u: %s-%u status=%s encoder=%u "
           "physical=%ux%u mm subpixel=%u\n",
           connector->connector_id, type_name ? type_name : "unknown",
           connector->connector_type_id, connection_name(connector->connection),
           connector->encoder_id, connector->mmWidth, connector->mmHeight,
           connector->subpixel);

    print_id_list("encoders", connector->encoders, connector->count_encoders);
    printf("    modes (%d):\n", connector->count_modes);
    for (int j = 0; j < connector->count_modes; j++) {
      print_mode(&connector->modes[j], "      ");
    }
    if (print_properties(fd, connector->props, connector->prop_values,
                         connector->count_props, "    ") < 0) {
      result = -1;
    }

    drmModeFreeConnector(connector);
  }

  return result;
}

static void format_name(uint32_t format, char name[5]) {
  for (unsigned int i = 0; i < 4; i++) {
    unsigned char byte = (format >> (i * 8)) & 0xff;

    name[i] = isprint(byte) ? (char)byte : '.';
  }
  name[4] = '\0';
}

static int print_planes(int fd) {
  drmModePlaneResPtr resources;
  int result = 0;

  if (drmSetClientCap(fd, DRM_CLIENT_CAP_UNIVERSAL_PLANES, 1) < 0) {
    fprintf(stderr,
            "warning: cannot enable universal planes: %s; "
            "showing legacy planes only\n",
            strerror(errno));
  }

  resources = drmModeGetPlaneResources(fd);
  if (!resources) {
    fprintf(stderr, "cannot get plane resources: %s\n", strerror(errno));
    return -1;
  }

  printf("\nPlanes (%u):\n", resources->count_planes);
  for (uint32_t i = 0; i < resources->count_planes; i++) {
    drmModePlanePtr plane = drmModeGetPlane(fd, resources->planes[i]);

    if (!plane) {
      fprintf(stderr, "  plane %u: %s\n", resources->planes[i],
              strerror(errno));
      result = -1;
      continue;
    }

    printf("  plane %u: crtc=%u fb=%u possible_crtcs=%#x "
           "gamma=%u\n",
           plane->plane_id, plane->crtc_id, plane->fb_id, plane->possible_crtcs,
           plane->gamma_size);
    printf("    formats (%u):", plane->count_formats);
    for (uint32_t j = 0; j < plane->count_formats; j++) {
      char name[5];

      format_name(plane->formats[j], name);
      printf(" %s(%#x)", name, plane->formats[j]);
    }
    putchar('\n');
    if (print_object_properties(fd, plane->plane_id, DRM_MODE_OBJECT_PLANE,
                                "    ") < 0) {
      result = -1;
    }
    drmModeFreePlane(plane);
  }

  drmModeFreePlaneResources(resources);
  return result;
}

int main(int argc, char **argv) {
  const char *device = DEFAULT_DRM_DEVICE;
  drmModeResPtr resources;
  int result = EXIT_SUCCESS;
  int fd;

  if (argc > 2) {
    fprintf(stderr, "usage: %s [/dev/dri/cardN]\n", argv[0]);
    return EXIT_FAILURE;
  }
  if (argc == 2) {
    device = argv[1];
  }

  fd = open(device, O_RDWR);
  if (fd < 0) {
    fprintf(stderr, "cannot open %s: %s\n", device, strerror(errno));
    return EXIT_FAILURE;
  }

  printf("DRM device node: %s\n", device);
  if (print_device_version(fd) < 0) {
    result = EXIT_FAILURE;
  }

  resources = drmModeGetResources(fd);
  if (!resources) {
    fprintf(stderr, "cannot get KMS resources from %s: %s\n", device,
            strerror(errno));
    close(fd);
    return EXIT_FAILURE;
  }

  printf("Framebuffer limits: %ux%u .. %ux%u\n", resources->min_width,
         resources->min_height, resources->max_width, resources->max_height);
  print_id_list("framebuffers", resources->fbs, resources->count_fbs);
  print_id_list("CRTCs", resources->crtcs, resources->count_crtcs);
  print_id_list("connectors", resources->connectors,
                resources->count_connectors);
  print_id_list("encoders", resources->encoders, resources->count_encoders);

  if (print_crtcs(fd, resources) < 0) {
    result = EXIT_FAILURE;
  }
  if (print_encoders(fd, resources) < 0) {
    result = EXIT_FAILURE;
  }
  if (print_connectors(fd, resources) < 0) {
    result = EXIT_FAILURE;
  }
  if (print_planes(fd) < 0) {
    result = EXIT_FAILURE;
  }

  drmModeFreeResources(resources);
  if (close(fd) < 0) {
    fprintf(stderr, "cannot close %s: %s\n", device, strerror(errno));
    result = EXIT_FAILURE;
  }
  return result;
}
