# SPDX-License-Identifier: MPL-2.0

import re
import argparse
import os
import sys
import logging

# Setup logging
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

def validate_out_dir(out_dir):
    if os.path.isabs(out_dir):
        print("Error: The --out-dir argument must be a relative path.")
        sys.exit(1)

def setup_output_directory(out_dir):
    template_dir = os.path.dirname(os.path.abspath(__file__))
    if out_dir == '.':
        return template_dir
    output_directory_path = os.path.join(template_dir, out_dir)
    if not os.path.exists(output_directory_path):
        os.makedirs(output_directory_path)
    return output_directory_path

def load_template(template_dir):
    template_file = os.path.join(template_dir, 'Dockerfile.template')
    if not os.path.isfile(template_file):
        logging.error(f"Template file {template_file} does not exist.")
        sys.exit(1)
    with open(template_file, 'r') as file:
        return file.read()

def generate_dockerfile_content(variables, template_content):
    for var_name, var_value in variables.items():
        template_content = re.sub(r'{%\s*' + var_name + r'\s*%}', var_value, template_content)
    return template_content

def write_dockerfile(output_directory, content):
    output_path = os.path.join(output_directory, 'Dockerfile')
    with open(output_path, 'w') as file:
        file.write(content)
    logging.info(f'Dockerfile has been generated at {output_path}.')

def main():
    args = parse_arguments()
    validate_out_dir(args.out_dir)

    variables = {
        'base_image': r'ubuntu:22.04',
        'qemu_ovmf_installation': r"""ovmf \ 
    qemu-system-x86""",
    }

    if args.intel_tdx:
        variables['base_image'] = r'intelcczoo/tdvm:ubuntu22.04-mvp_2023ww15'
        variables['qemu_ovmf_installation'] = r''

    template_dir = os.path.dirname(os.path.abspath(__file__))
    output_directory = setup_output_directory(args.out_dir)
    template_content = load_template(template_dir)
    dockerfile_content = generate_dockerfile_content(variables, template_content)
    write_dockerfile(output_directory, dockerfile_content)

if __name__ == '__main__':
    main()
