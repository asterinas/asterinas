# SPDX-License-Identifier: MPL-2.0

import argparse
import os
import sys
import logging
from jinja2 import Environment, FileSystemLoader

logging.basicConfig(level=logging.INFO, format='%(levelname)s: %(message)s')

def parse_arguments():
    parser = argparse.ArgumentParser(description='The Dockerfile generator for OSDK.')
    parser.add_argument('--intel-tdx', action='store_true', help='Include Intel TDX support')
    parser.add_argument(
        '--out-dir',
        type=str,
        default='.',
        help='Output the Dockerfile under this directory. \
            By default, the output directory is the current working directory.'
    )
    return parser.parse_args()

def setup_output_directory(out_dir):
    if os.path.isabs(out_dir):
        logging.error("The --out-dir argument must be a relative path.")
        sys.exit(1)
    template_dir = os.path.dirname(os.path.abspath(__file__))
    if out_dir == '.':
        return template_dir
    output_directory_path = os.path.join(template_dir, out_dir)
    if not os.path.exists(output_directory_path):
        os.makedirs(output_directory_path)
    return output_directory_path

def load_template():
    template_dir = os.path.dirname(os.path.abspath(__file__))
    env = Environment(loader=FileSystemLoader(template_dir), trim_blocks=True, lstrip_blocks=True)
    template = env.get_template('Dockerfile.jinja')
    return template

def write_dockerfile(output_directory, content):
    output_path = os.path.join(output_directory, 'Dockerfile')
    with open(output_path, 'w') as file:
        file.write(content)
    logging.info(f'Dockerfile has been generated at {output_path}.')

def main():
    args = parse_arguments()
    output_dir = setup_output_directory(args.out_dir)
    base_image = "intelcczoo/tdvm:ubuntu22.04-mvp_2023ww15" if args.intel_tdx else "ubuntu:22.04"

    template = load_template()
    rendered_content = template.render(base_image=base_image, intel_tdx=args.intel_tdx)

    write_dockerfile(output_dir, rendered_content)

if __name__ == '__main__':
    main()
