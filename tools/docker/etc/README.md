### About the etc directory

The `etc` directory contains various configuration files used for system settings. The whole directory will be copied to `/opt/jinux/etc` when building docker image. Then these files can be used to creating initramfs for jinux, if needed.

Below describes the format of files in this directory.

### `group`

The `group` file is a text file to store information about groups. Each line represents a group and is separated by colons (":") into multiple fields.
Here's how to interpret each field in a line:
- **Group Name**: This field represents the name of the group and serves as a unique identifier for the group.
- **Group Password**: Similar to the password field in `passwd`, this field is often encrypted or displayed as a placeholder in modern systems. The actual group password is stored in the `gshadow` file.
- **Group ID (GID)**: Each group has a unique numeric ID used to identify the group.
- **Group Members**: This field lists the users who belong to the group, separated by commas (,).

### `passwd`

Each line in the `passwd` file represents a user account and is formatted with several fields separated by colons (":").
Here's how to interpret each field in a line:
- **Username**: This field represents the login name of the user and serves as a unique identifier for the user.
- **Password**:  In the `passwd` file, this field may be displayed as a placeholder, such as "x" or "*". The actual password is stored in the `shadow` file.
- **User ID (UID)**: Each user has a unique numeric ID used to identify the user. 
- **Group ID (GID)**: This field indicates the primary group ID to which the user belongs. It specifies the group with which the user is associated.
- **User Information**: This field may contain additional user information, such as the user's full name, contact details, etc.
- **Home Directory**: This field represents the path to the user's home directory, where the user is placed after logging in.
- **Login Shell**: This field specifies the path to the shell program that the user will use after logging in. The shell is the primary interface through which the user interacts with the operating system.