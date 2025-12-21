This repo contains the Controllers (firmware, pcb design, printables) that make up part of the Makerspace Access Control System.

The basic principles they follow are:

* Use NFC readers (enrol a fob or card of your choice)
* Receive updates over Wifi, but cached on the controller in flash storage.
* Able to work offline and queue events to send to controller when connectivity established
* Provide log updates to the access controller backend to allow device usage to be logged and tracked.

They're an important part of our Health and Safety obligations to our members, ensuring that only authorised users are able to use (potentially) dangerous hardware.

The documentation is on the [wiki (in progress...!)](https://github.com/MakerSpaceNewcastle/access-control-gen2/wiki)
