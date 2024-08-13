# Boterinas 

## Introduction

`@boterinas` is a general-purpose bot designed for a wide variety of tasks in Asterinas. It streamlines maintenance tasks to enhance workflow efficiency. 

Commands are issued by writing comments that start with the text `@boterinas`. The available commands depend on which repository you are using. The main Asterinas repository contains a `triagebot.toml` file where you can see which features are enabled. 

Commands for GitHub issues or pull requests should be issued by writing `@boterinas` followed by the command anywhere in the comment. Note that `@boterinas` will ignore commands in Markdown code blocks, inline code spans, or blockquotes. You can enter multiple `@boterinas` commands in a single comment. 

For example, you can claim an issue and add a label in the same comment.
```markdown
@boterinas claim
@boterinas label C-enhancement
```

Additionally, `@boterinas` allows for editing comments. If you don't change the text of a command, the edit will be ignored. However, if you modify an existing command or add new ones, those commands will be processed.

Below, you'll find a comprehensive guide on how to use `@boterinas` effectively.

## Commands and Usage

### Workflow Management
- **`@boterinas rerun`**  
  Restarts the workflow of the current pull request if it has failed unexpectedly. Only the author of the pull request can use this command.

### Issue and Pull Request Management
- **`@boterinas claim`**  
  Assigns the issue or pull request to yourself.  
  
- **`@boterinas release-assignment`**  
  Removes the current assignee from an issue or pull request. This command can only be executed by the current assignee or a team member.  
  
- **`@boterinas assign @user`**  
  Assigns a specific user to the issue or pull request. Only team members have permission to assign other users.  

### Label Management
- **`@boterinas label <label>`**  
  Adds a label to the issue or pull request.  
  *Example:* `@boterinas label C-enhancement C-rfc`
  
- **`@boterinas label -<label>`**  
  Removes a label from the issue or pull request.  
  *Example:* `@boterinas label -C-enhancement -C-bug`

### Status Indicators
- **`@boterinas author`**  
  Indicates that a pull request is waiting on the author. It assigns the `S-waiting-on-author` label and removes both `S-waiting-on-review` and `S-blocked`, if present.  
  
- **`@boterinas blocked`**  
  Marks a pull request as blocked on something.  
  
- **`@boterinas ready`**  
  Indicates that a pull request is ready for review. This command can also be invoked with the aliases `@boterinas review` or `@boterinas reviewer`.  

## Notes
- Only team members can assign users or remove assignments.
- Labels are crucial for organizing issues and pull requests, so ensure they are used consistently and accurately.
- For any issues or questions regarding `@boterinas`, please reach out to the team for support.
