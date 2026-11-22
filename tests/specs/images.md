# Images

What Sherpa does with images themselves: importing and uploading VM and unikernel disk images, downloading a VM image, pulling container images, scanning for images that are on the server but not recorded, the default version of each model and kind, listing, showing, editing and deleting images, who may do each, and which image up and redeploy give a node. How a node boots from its image is in [nodes](nodes.md); the lab-level refusal of a lab whose image is missing is `lab_preflight_001` in [lab lifecycle](lab-lifecycle.md); which kinds a model takes, an image's ZTP setting, and the install and update scripts, with the move of a renamed model's image folders after an update, have their own specs.

An image is a model, a kind (virtual machine, container or unikernel) and a version.

Each behaviour lists its cases. A case is one test, whose name is its ID: `<behaviour>_<NNN>_<description>`, such as `image_delete_002_a_container_image_a_running_container_runs_is_not_deleted`. The level is the highest that can show the case: e2e through the `sherpa` CLI, integration against real libvirt, Docker, the database or a running sherpad, unit only for logic with too many branches to cover above.

## `image_import`: an imported disk image is on the server whole, or not at all

**Statement:** importing or uploading an image puts the whole file on the server and records it under a version that is a plain name, never a path, or leaves the server as it was.
**Relied on by:** the admin, every lab built from the image, the server's files outside the images directory. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | An imported disk is in the images directory under its model and version, and recorded | integration |
| 002 | A source file that does not exist is refused, and nothing is recorded | integration |
| 003 | A source that cannot be read through leaves no file, no partial file and no record | integration |
| 004 | An import whose record cannot be written removes the file it added | integration |
| 005 | An import that replaced a file and whose record cannot be written keeps the new file, which the existing record names | unit |
| 006 | The image is at its path only once it is whole: until then nothing is there, or the file that was | unit |
| 007 | An import that is interrupted leaves no partial file | unit |
| 008 | Importing a version whose file on the server has the source's size skips the copy | integration |
| 009 | Importing a version whose file on the server has another size replaces it | integration |
| 010 | Importing a version whose file has another size and has lab disks built on it is refused, and the file is as it was | integration |
| 011 | Importing a version again keeps the settings an admin changed for it | integration |
| 012 | A VM and a unikernel of one model imported at the same version each keep their own file in the one version folder (no test yet) | integration |
| 013 | An import with `--kind container` loads a tar archive into Docker and records the version (no test yet) | integration |
| 014 | A container archive that cannot be read through is a failed import | integration |
| 015 | An import request with a field the server does not take, such as an image setting an older client sends, is refused naming its path | unit |
| 016 | An upload, in the web UI or by `POST /api/v1/images/upload`, imports its file as import does (no test yet) | integration |
| 017 | An upload without its model, version or file, with an empty version, an unknown model or a kind the model does not take, is refused saying which | unit |
| 018 | An upload without a kind takes the model's default kind, and with one takes that kind | unit |
| 019 | The upload page offers each model its kinds with its default chosen, and asks for a kind where the model has no default | unit |
| 020 | An image imported with `sherpa server image import` is listed, and a lab comes up from it (no test yet) | e2e |
| 021 | A version is 1 to 64 letters, digits, `.`, `_`, `+` or `-`, starting with a letter or digit: versions vendors use, such as `24.4`, `9-23-1` and `26.2R1.7-EVO`, are taken | unit |
| 022 | An empty version, one over 64 characters, one starting with another character, and one with `/`, `..` or any other character is refused, so a version is never a path | unit |
| 023 | Import, pull, download and delete refuse such a version before they touch a file, Docker or the database (no test yet) | integration |

## `image_download`: a VM image is downloaded whole from a URL

**Statement:** a VM image is downloaded whole from a URL and recorded, or nothing is left at its path; a file already at the path is kept only when it matches what the source says the image is.
**Relied on by:** the admin. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | A complete download is at the image's path and recorded (no test at this level yet) | integration |
| 002 | A download that breaks off leaves nothing at the path, so the next run downloads again | unit |
| 003 | A body shorter than the length the server declared is refused | unit |
| 004 | An HTTP answer that is not a success is refused, and nothing is recorded (no test yet) | integration |
| 005 | A version whose file on the server has the length the source declares, and the SHA-256 it publishes where it publishes one, is recorded without a download | unit |
| 006 | `--url` is the URL downloaded, for any model | unit |
| 007 | Without `--url` only `ubuntu_linux` and `talos_linux` are downloaded; any other model is refused, saying to give `--url` | unit |
| 008 | A `talos_linux` version, with or without a leading `v`, is downloaded from the Talos Image Factory's vanilla schematic | unit |
| 009 | An Image Factory answer without a schematic ID is refused | unit |
| 010 | The Talos Image Factory answers Sherpa's request with the vanilla schematic's ID | integration |
| 011 | Only a virtual machine image is downloaded: a unikernel is refused, and so is a download without `--version` (no test yet) | unit |
| 012 | A file on the server shorter than the length the source declares, such as a partial file an interrupted download left, is downloaded again and replaced | unit |
| 013 | A file of the source's length whose SHA-256 is not the one the source publishes is downloaded again and replaced | unit |
| 014 | A file is downloaded again and replaced when the source declares no length, as the Talos Image Factory and a `--url` may not | unit |
| 015 | A download whose SHA-256 is not the one the source publishes is refused, leaving nothing at the path and no record | unit |
| 016 | An Ubuntu download whose `SHA256SUMS` cannot be fetched, or does not list the image, is refused | unit |
| 017 | A file to be replaced that has lab disks built on it is refused, and the file is as it was | unit |

Only an Ubuntu release, downloaded without `--url`, publishes a SHA-256 (its `SHA256SUMS`); the Talos Image Factory and a `--url` are checked by length alone.

## `image_pull`: a pulled container image is in Docker and recorded with its repo

**Statement:** a container image pulled through Sherpa is in local Docker and recorded with its tag as the version and its repo as written.
**Relied on by:** the admin, container nodes of that version. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | A pull brings the image into local Docker and records its model, version and repo | integration |
| 002 | The repo is recorded as it was written, for a model with a built-in repo too | unit |
| 003 | A version pulled again from another repo is recorded with the repo pulled last (no test yet) | integration |
| 004 | `--repo` is split at its tag: a short name, a full Docker Hub name, and a registry with a port | unit |
| 005 | A `--repo` with no tag, an empty tag or a digest is refused, naming the forms that are taken | unit |
| 006 | Without `--repo`, a model with a known public repo is pulled from it at `--version`; any other model is refused, saying to give `--repo` (no test yet) | unit |
| 007 | A pull for a model or kind that is not a container is refused by the server (no test yet) | integration |
| 008 | A pull that fails records nothing (no test yet) | integration |

## `image_scan`: scan records the images on the server that are not recorded

**Statement:** a scan records each image that is on the server but not recorded, and leaves recorded ones as they are.
**Relied on by:** the admin. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | A VM or unikernel disk in the images directory that is not recorded is recorded under its model, kind and version (no test yet) | integration |
| 002 | A version already recorded is reported `already_exists` and left as it is | integration |
| 003 | `--dry-run` reports what would be recorded and records nothing | integration |
| 004 | With `--model` and `--repo`, each local tag of that repo is recorded as a version of the model, with the repo | integration |
| 005 | Without them, each container model with a built-in repo gets that repo's local tags, and the container models with none are listed as skipped | integration |
| 006 | A Docker Hub repo is matched by its short name and by `docker.io/library/<name>`, and by no other spelling | integration |
| 007 | A repo never matches a repo it is the start or the end of | integration |
| 008 | A repo given with a tag is refused, saying how to write it | integration |
| 009 | `--model` and `--repo` are refused unless both are given | unit |
| 010 | `--model` and `--repo` are for containers: a model with no container kind, or another `--kind`, is refused | unit |
| 011 | A repo that is not a valid repository name is refused | unit |
| 012 | The CLI refuses `--model` or `--repo` alone, or without `--kind container`, before it asks the server | unit |
| 013 | Each tag Docker lists is a version, whatever name Docker lists the repo by, a registry port included | unit |
| 014 | An image with no tag, and a tag that is not a valid version, is left out | unit |
| 015 | With `--model` and `--repo` no other repo is scanned | unit |
| 016 | A scan that waited for a delete of a version does not record the deleted image again | integration |
| 017 | A folder that is not a model's, and a symlinked version folder, is skipped (no test yet) | integration |
| 018 | `--kind` scans that kind only (no test yet) | integration |
| 019 | The CLI lists each image found with its status, and each skipped model with the reason | unit |

## `image_default`: each model and kind with images has one default version

**Statement:** each model and kind with images has exactly one default version.
**Relied on by:** the user, whose node without a `version` gets it; the admin. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | The first version recorded for a model and kind becomes its default without being asked | integration |
| 002 | `--default` makes the new version the default, and the old default no longer is | integration |
| 003 | Recording a version again without `--default` leaves the default where it was, also when that version is the default | integration |
| 004 | `set-default` moves the default to the version it names | integration |
| 005 | `set-default` of a version that is not on the server is refused (no test yet) | integration |
| 006 | The web form cannot unset the default: it is refused, saying to make another version the default | integration |
| 007 | Deleting the default makes the most recently added remaining version the default, and reports which | integration |
| 008 | Deleting a version that is not the default, or a model's last version, changes no default | integration |
| 009 | Two defaults for one model and kind are never stored | integration |
| 010 | Each kind of a model has its own default: making a container version the default leaves the virtual machine's | integration |
| 011 | A scan makes the first version it finds the default only for a model and kind with no default, and a dry run says so | integration |

## `image_delete`: delete removes an image, unless something uses it

**Statement:** delete removes an image's record and its file or Docker image, and is refused while a lab disk, a running container or a node uses the image.
**Relied on by:** the admin, every lab built from the image. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | Deleting a container image removes its record and its repo at its version from Docker, and keeps another tag of the same image | integration |
| 002 | A container image a running container runs is not deleted: the refusal names the containers, and the record and the image are kept | integration |
| 003 | A container created from another tag of the same image does not stop the delete, unless the tag being deleted is the image's last | unit |
| 004 | Deleting a VM or unikernel image removes its file, and the version folder with the last file in it (no test at this level yet) | integration |
| 005 | Deleting one kind of a model at a version keeps the other kind's record and file at that version | integration |
| 006 | A VM or unikernel image with a lab disk built on it, running or stopped, is not deleted, and the refusal names the disk (no test at this level yet) | integration |
| 007 | A lab disk built on another image does not stop the delete | unit |
| 008 | A lab disk copied from the image (`clone_method = "copy"`) does not stop the delete; only an overlay does (no test at this level yet) | integration |
| 009 | A lab disk whose base is reached through a renamed model's old folder still stops the delete | unit |
| 010 | When the lab disks cannot be read, or a disk's base cannot be found, the delete is refused | unit |
| 011 | An image a lab node records is not deleted | integration |
| 012 | Once a node is redeployed onto another version, the old image can be deleted and the new one cannot | integration |
| 013 | A delete whose record cannot be removed keeps the image's file | unit |
| 014 | An image whose file is already gone is still deleted, with its empty folder | unit |
| 015 | Deleting an image that is not on the server is refused, saying to list the images (no test yet) | integration |
| 016 | A delete waits for an up that is building a disk on the image, and is then refused by that disk | unit |
| 017 | Operations on one image version run one at a time, and another version is not held up | unit |
| 018 | An up whose image was deleted before it built anything is refused, with nothing created (no test yet) | integration |

## `image_list`: list and show report the recorded images

**Statement:** list and show report the recorded images, each named by its model, kind and version.
**Relied on by:** the user, the admin, scripts reading the JSON. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | List gives each image's model, kind and version, and whether it is the default | integration |
| 002 | List takes a model, a kind or both, and gives only those images (no test yet) | integration |
| 003 | Show without a version gives the default version's settings, and with one that version's | integration |
| 004 | Show of a model, kind or version with no image is refused, saying none was found (no test yet) | integration |
| 005 | A model with two kinds has its image of each kind shown, made default and edited under that kind, in the API and the admin pages | integration |
| 006 | A kind the model does not take is refused as a bad request, naming the kind | integration |
| 007 | A path whose kind is not a kind is refused as a bad request | integration |
| 008 | `image list` prints a table of each image's model, kind, version and default | unit |
| 009 | `image show` prints a row for each setting, named by its key in the image's JSON: the settings every kind takes, then its own kind's group | unit |
| 010 | Without `--kind` a command takes the model's default kind, and for `generic`, which has none, is refused naming its kinds (no test yet) | integration |
| 011 | The versions of a model and kind are listed in version order | integration |

## `image_settings`: an admin's edit changes one image's settings

**Statement:** an admin's edit in the web UI changes the settings of one image version, and nothing else about it.
**Relied on by:** the admin, nodes built from the image afterwards. **Priority:** P2.

| Case | What it shows | Level |
|---|---|---|
| 001 | A form saved unchanged leaves an image of each kind as it was | unit |
| 002 | A changed CPU count, memory and interface count are stored, under the same image | integration |
| 003 | A node built after an edit runs with the edited settings (no test yet) | integration |
| 004 | A form with a field of another kind is refused naming the field | unit |
| 005 | A form without a field its kind needs is refused naming the field | unit |
| 006 | A form with a field the server does not take, as a page loaded from an older server sends, is refused naming the field, not saved without it | unit |
| 007 | An edit cannot change the image's model, kind, version, management interface or, for a unikernel, boot mode | unit |
| 008 | A CPU count under 1, memory under 64 MB or an interface MTU outside 576 to 9600 is refused naming the setting, and values in range are taken | unit |
| 009 | A container image has no interface MTU to check | unit |
| 010 | The edit page shows the image's own settings chosen, and names its kind | unit |
| 011 | The image page shows each kind only the settings it takes | unit |
| 012 | Every model's built-in settings are recorded and read back unchanged | integration |

The ZTP setting of an image, and which methods a model and kind take, belong to the ZTP spec.

## `image_selection`: each node gets the image of its model, kind and version

**Statement:** up and redeploy give each node the image of its model, kind and version, the default version when it names none, or refuse before anything is created.
**Relied on by:** the user. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | A node with a `version` gets that version, not the default | unit |
| 002 | A node without a `version` gets its model and kind's default version | unit |
| 003 | A node gets an image of its own kind only: the same version of the model's other kind is never taken, default or not | unit |
| 004 | A model and kind with no image is refused, naming the model, the kind and the node | unit |
| 005 | A version that is not on the server is refused, listing the versions that are | unit |
| 006 | A VM image that is recorded but whose file is gone is refused, naming the path it should be at | unit |
| 007 | A unikernel's image is looked for under the file name of its boot mode | unit |
| 008 | A container node runs the manifest's `image` over the repo recorded for its version, for a model with a built-in repo too | unit |
| 009 | A container node without `image` runs the repo recorded for its version | unit |
| 010 | A container node with neither is refused, naming both ways to give it one | unit |
| 011 | The recorded repo is used as written: a full Docker Hub name, a short one, or another registry's path | unit |
| 012 | A container node whose image is not in local Docker is refused before anything is created, saying how to pull it (no test yet) | integration |
| 013 | An official Docker Hub image pulled by its full name is found, and its node comes up | integration |
| 014 | A redeploy rebuilds a node from the manifest's version of its model and kind, else their default, and refuses a version only the other kind has | integration |

`lab_preflight_001` in [lab lifecycle](lab-lifecycle.md) shows the refusal of a missing image through up, with nothing left behind.

## `image_access`: any user reads images, only an admin changes them

**Statement:** any logged-in user can list and show images, and only an admin can change them.
**Relied on by:** the admin, the user. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | Listing and showing images is refused without a login, and answered for a user who is not an admin | integration |
| 002 | Scan, import, upload, pull, download, delete and set-default are refused to a user who is not an admin, over RPC and HTTP | integration |
| 003 | The admin image pages are refused to a user who is not an admin (no test with its ID yet; `admin_pages_are_refused_to_others_and_served_to_admins` covers them) | integration |
| 004 | A user who is not an admin runs `sherpa image list` and `sherpa image show`, and is refused `sherpa server image` commands that change images (no test yet) | e2e |
