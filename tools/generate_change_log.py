#!/usr/bin/env python

import requests
import os
from datetime import datetime, timedelta

def search_prs_with_label():
    url = f"https://api.github.com/repos/asterinas/asterinas/pulls"

    params = {
        "state": "closed",
        "sort": "merged_at",
        "direction": "desc",
    }

    response = requests.get(url, params=params)
    prs = response.json()
    pr_lines = []

    # TODO: get PRs since a tag, instead of a starting time
    today = datetime.today()
    last_month = today - timedelta(days=30)

    # Get PRs one page per time
    while True:
        for pr in prs:
            if pr["merged_at"] is None:
                continue
            merged_date = datetime.strptime(pr["merged_at"], "%Y-%m-%dT%H:%M:%SZ").date()
            if merged_date < last_month.date():
                response = None
                break
            pr_number = pr["number"]
            pr_title = pr["title"]
            pr_url = pr["url"]
            line = "- {titie}([#{number}]({url}))\n".format(titie = pr_title, number = pr_number, url = pr_url)
            pr_lines.append(line)
        
        if response is None:
            break
        
        if 'next' in response.links:
            next_url = response.links['next']['url']
            print(next_url)
            response = requests.get(next_url)
            prs = response.json()
        else:
            break
    return pr_lines

if __name__ == "__main__":
    current_script_dir = os.path.dirname(os.path.abspath(__file__))
    changelog_path = os.path.join(current_script_dir, "..", "CHANGELOG.md")
    print("Generate changelog in", changelog_path, "......")

    with open(changelog_path, "r") as file:
        changelog_lines = file.readlines()

    # Finds the line contains "No unrelased changes yet."
    index = 0
    for i in range(len(changelog_lines)):
        if changelog_lines[i].find("No unrelased changes yet.") != -1:
            index = i
            break
    
    if index == 0:
        print("fails to find unrelease section in CHANGELOG.md")
        exit(-1)

    # Reads version from the file
    version_path = os.path.join(current_script_dir, "..", "VERSION")
    with open(version_path, "r") as file:
        version = file.read()
    version = version.strip()

    # Checks the version does not exist
    for line in changelog_lines:
        if line.startswith("##") and line.find(version) != -1:
            print("the version", version, "is already used")
            exit(-1)

    pr_lines = search_prs_with_label()

    # TODO: adds link to the release in the version line
    version_line = "## v{version}\n".format(version = version)
    with open(changelog_path, "w") as file:
        file.writelines(changelog_lines[:index + 1])
        file.write('\n')
        file.write(version_line)
        file.writelines(pr_lines)
        file.writelines(changelog_lines[index + 1:])

    print("Update the content of", changelog_path)
